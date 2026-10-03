// SPDX-License-Identifier: GPL-2.0-only

//! Preview road-tool Godot API methods.

use super::super::super::*;
use crate::simulation::network::surface::RoadPreviewVisualMesh;

#[godot_api(secondary)]
impl SimulationNode {
    /// Returns cheap synchronous hover feedback for a road-tool candidate.
    ///
    /// Returns prepared points and validation without allocating a display ribbon. Call
    /// `build_road_preview_ribbon` only when fallback geometry will actually be displayed.
    #[func]
    pub fn validate_road_candidate(
        &self,
        points: PackedVector3Array,
        fwd_lanes: i32,
        bkw_lanes: i32,
    ) -> Variant {
        self.validate_road_candidate_with_snap(points, fwd_lanes, bkw_lanes, true)
    }

    /// Returns cheap synchronous hover feedback with optional existing-road snapping.
    #[func]
    pub fn validate_road_candidate_with_snap(
        &self,
        points: PackedVector3Array,
        fwd_lanes: i32,
        bkw_lanes: i32,
        snap_to_existing_roads: bool,
    ) -> Variant {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let point_count = points.len();
        let clone_start = road_debug.then(Instant::now);
        let points = points.to_vec();
        let clone_ms = clone_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let validate_start = road_debug.then(Instant::now);
        let (prepared_points, validation, surface_generation) = {
            let query = self.road_tool_query_snapshot.read().unwrap();
            let prepared_input = RoadSurfaceSystem::prepare_road_input_for_tool(
                &points,
                &query.terrain,
                &query.region_graph,
                &query.road_surface,
                snap_to_existing_roads,
            );
            let validation = query.road_surface.validate_prepared_road_candidate_fast(
                &prepared_input,
                fwd_lanes.clamp(0, i32::from(u8::MAX)) as u8,
                bkw_lanes.clamp(0, i32::from(u8::MAX)) as u8,
                &query.terrain,
                &query.region_graph,
            );
            let validation = crate::nodes::sim::road_tool::validate_road_candidate_against_water(
                prepared_input.class,
                &prepared_input.points,
                fwd_lanes.clamp(0, i32::from(u8::MAX)) as u8,
                bkw_lanes.clamp(0, i32::from(u8::MAX)) as u8,
                &query.water,
                validation,
            );
            (prepared_input.points, validation, query.surface_generation)
        };
        let validate_ms = validate_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        if road_debug {
            debug_log!(
                "road",
                "road_candidate_fast_validation points={} prepared_points={} fwd_lanes={} bkw_lanes={} valid={} reason={} max_grade={:.3} allowed_grade={:.3} endpoint_snap=({},{}) clone_ms={:.3} validate_ms={:.3} total_ms={:.3}",
                point_count,
                prepared_points.len(),
                fwd_lanes,
                bkw_lanes,
                validation.is_valid,
                validation.invalid_reason,
                validation.max_grade,
                validation.allowed_grade,
                validation.start_endpoint_snapped_node_id,
                validation.end_endpoint_snapped_node_id,
                clone_ms,
                validate_ms,
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }

        let result = self.road_candidate_validation_to_variant(
            &validation,
            &prepared_points,
            fwd_lanes,
            bkw_lanes,
        );
        let mut dict = result.to::<VarDictionary>();
        dict.set(
            "surface_generation",
            i64::try_from(surface_generation).unwrap_or(i64::MAX),
        );
        dict.to_variant()
    }

    /// Builds display-only fallback geometry from an already-prepared road profile.
    ///
    /// Returns nil if the source terrain/road revision changed. Does not snap or validate again;
    /// cost is O(profile samples × lateral strips), with constant-time terrain-grid sampling.
    /// The lifted vertices are never authoritative placement inputs.
    #[func]
    pub fn build_road_preview_ribbon(
        &self,
        prepared_points: PackedVector3Array,
        fwd_lanes: i32,
        bkw_lanes: i32,
        surface_generation: i64,
    ) -> Variant {
        let query = self.road_tool_query_snapshot.read().unwrap().clone();
        if i64::try_from(query.surface_generation).unwrap_or(i64::MAX) != surface_generation {
            return Variant::nil();
        }
        let mesh = query.road_surface.build_preview_visual_mesh(
            &prepared_points.to_vec(),
            &[],
            fwd_lanes.clamp(0, i32::from(u8::MAX)) as u8,
            bkw_lanes.clamp(0, i32::from(u8::MAX)) as u8,
            &query.terrain,
        );
        let mut dict = VarDictionary::new();
        dict.set("surface_generation", surface_generation);
        Self::append_road_preview_visual_mesh(&mut dict, &mesh);
        dict.to_variant()
    }

    /// Validates a road-tool candidate by compiling temporary surface geometry with optional snap.
    #[func]
    pub fn validate_road_candidate_for_commit_with_snap(
        &self,
        points: PackedVector3Array,
        fwd_lanes: i32,
        bkw_lanes: i32,
        snap_to_existing_roads: bool,
    ) -> Variant {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let point_count = points.len();
        let clone_start = road_debug.then(Instant::now);
        let points = points.to_vec();
        let clone_ms = clone_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let validate_start = road_debug.then(Instant::now);
        let fwd_lanes_u8 = fwd_lanes.clamp(0, i32::from(u8::MAX)) as u8;
        let bkw_lanes_u8 = bkw_lanes.clamp(0, i32::from(u8::MAX)) as u8;
        let (prepared_points, validation, surface_generation) = {
            let query = self.road_tool_query_snapshot.read().unwrap();
            let prepared_input = RoadSurfaceSystem::prepare_road_input_for_tool(
                &points,
                &query.terrain,
                &query.region_graph,
                &query.road_surface,
                snap_to_existing_roads,
            );
            let new_edge_validation = query.road_surface.validate_prepared_road_surface(
                &prepared_input.points,
                prepared_input.class,
                fwd_lanes_u8,
                bkw_lanes_u8,
                &query.terrain,
            );
            let validation = query
                .road_surface
                .validate_prepared_road_input_against_graph_with_compile_reason(
                    &prepared_input,
                    fwd_lanes_u8,
                    bkw_lanes_u8,
                    &query.terrain,
                    &query.region_graph,
                    new_edge_validation,
                    RoadSurfaceCompileReason::CommitValidator,
                );
            let validation = crate::nodes::sim::road_tool::validate_road_candidate_against_water(
                prepared_input.class,
                &prepared_input.points,
                fwd_lanes_u8,
                bkw_lanes_u8,
                &query.water,
                validation,
            );
            (prepared_input.points, validation, query.surface_generation)
        };
        let validate_ms = validate_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        if road_debug {
            debug_log!(
                "road",
                "road_candidate_commit_validation points={} prepared_points={} fwd_lanes={} bkw_lanes={} valid={} reason={} max_grade={:.3} allowed_grade={:.3} endpoint_snap=({},{}) clone_ms={:.3} validate_ms={:.3} total_ms={:.3}",
                point_count,
                prepared_points.len(),
                fwd_lanes,
                bkw_lanes,
                validation.is_valid,
                validation.invalid_reason,
                validation.max_grade,
                validation.allowed_grade,
                validation.start_endpoint_snapped_node_id,
                validation.end_endpoint_snapped_node_id,
                clone_ms,
                validate_ms,
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }

        let mut dict = self
            .road_candidate_validation_to_variant(
                &validation,
                &prepared_points,
                fwd_lanes,
                bkw_lanes,
            )
            .to::<VarDictionary>();
        dict.set(
            "surface_generation",
            i64::try_from(surface_generation).unwrap_or(i64::MAX),
        );
        dict.to_variant()
    }

    /// Returns the road-tool surface snapshot generation currently used for validation.
    #[func]
    pub fn get_road_tool_surface_generation(&self) -> i64 {
        let query = self.road_tool_query_snapshot.read().unwrap();
        i64::try_from(query.surface_generation).unwrap_or(i64::MAX)
    }

    /// Requests temporary preview-surface compilation for the road tool.
    ///
    /// The result is published asynchronously through [`Self::get_preview_road_surface_result`]. The
    /// payload is visual-only; click validity is checked by the fast road-candidate validator.
    #[func]
    pub fn request_preview_road_surface(
        &self,
        points: PackedVector3Array,
        fwd_lanes: i32,
        bkw_lanes: i32,
    ) -> i64 {
        self.request_preview_road_surface_with_snap(points, fwd_lanes, bkw_lanes, true)
    }

    /// Requests temporary preview-surface compilation with optional existing-road snapping.
    #[func]
    pub fn request_preview_road_surface_with_snap(
        &self,
        points: PackedVector3Array,
        fwd_lanes: i32,
        bkw_lanes: i32,
        snap_to_existing_roads: bool,
    ) -> i64 {
        self.request_preview_road_surface_with_options(
            points,
            fwd_lanes,
            bkw_lanes,
            snap_to_existing_roads,
            false,
        )
    }

    /// Requests either road-only or complete road-and-terrain temporary geometry.
    #[func]
    pub fn request_preview_road_surface_with_options(
        &self,
        points: PackedVector3Array,
        fwd_lanes: i32,
        bkw_lanes: i32,
        snap_to_existing_roads: bool,
        include_terrain: bool,
    ) -> i64 {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let request_id = self
            .road_preview_request_counter
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        let point_count = points.len();
        let clone_start = road_debug.then(Instant::now);
        let points = points.to_vec();
        let clone_ms = clone_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let surface_generation = self
            .road_tool_query_snapshot
            .read()
            .expect("road query snapshot lock poisoned")
            .surface_generation;
        let send_start = road_debug.then(Instant::now);
        let send_ok = self.road_preview_tx.submit(RoadPreviewRequest {
            enqueued_at: None,
            include_terrain,
            request_id,
            surface_generation,
            points,
            fwd_lanes,
            bkw_lanes,
            snap_to_existing_roads,
        });
        let send_ms = send_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        if road_debug {
            debug_log!(
                "road",
                "preview_surface_request request_id={} points={} fwd_lanes={} bkw_lanes={} clone_ms={:.3} send_ms={:.3} send_ok={} total_ms={:.3}",
                request_id,
                point_count,
                fwd_lanes,
                bkw_lanes,
                clone_ms,
                send_ms,
                send_ok,
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }
        i64::try_from(request_id).unwrap_or(i64::MAX)
    }

    /// Latest preview request the worker has started, or zero. Every older request has already
    /// published or been abandoned; newer ones are pending or were displaced and never run.
    /// Read it before polling results to retire outstanding requests exactly, in O(1).
    #[func]
    pub fn get_preview_road_surface_started_request_id(&self) -> i64 {
        i64::try_from(self.road_preview_tx.started()).unwrap_or(i64::MAX)
    }

    /// Returns the completed road-tool preview for `request_id`, or `null` while pending/stale.
    /// `retained_revision` identifies retained geometry already installed by the caller; zero
    /// requests a complete payload. A matching revision omits unchanged retained mesh buffers.
    /// `terrain_revisions` lists preview terrain products the caller still holds; each matching
    /// patch is exported as metadata plus `unchanged`, and every other patch is complete.
    #[func]
    pub fn get_preview_road_surface_result(
        &self,
        request_id: i64,
        retained_revision: i64,
        terrain_revisions: PackedInt64Array,
    ) -> Variant {
        let Ok(request_id) = u64::try_from(request_id) else {
            return Variant::nil();
        };
        let result_lock_start = crate::debug::is_perf_enabled().then(Instant::now);
        let preview_result = self.road_preview_result.read().unwrap();
        let result_lock_ms = result_lock_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let Some(preview) = preview_result.as_ref() else {
            return Variant::nil();
        };
        if preview.request_id != request_id {
            return Variant::nil();
        }

        self.road_preview_snapshot_to_variant(
            preview,
            u64::try_from(retained_revision).unwrap_or(0),
            terrain_revisions.as_slice(),
            result_lock_ms,
        )
    }

    fn road_preview_snapshot_to_variant(
        &self,
        preview: &RoadPreviewSnapshot,
        retained_revision: u64,
        held_terrain: &[i64],
        result_lock_ms: f64,
    ) -> Variant {
        let export_start = preview.timing.as_ref().map(|_| Instant::now());
        let Some(mut dict) = self.road_candidate_dictionary_with_parcel_clearance(
            &preview.validation,
            &preview.prepared_points,
            i32::from(preview.fwd_lanes),
            i32::from(preview.bkw_lanes),
        ) else {
            return Variant::nil();
        };
        dict.set(
            "request_id",
            i64::try_from(preview.request_id).unwrap_or(i64::MAX),
        );
        dict.set(
            "surface_generation",
            i64::try_from(preview.surface_generation).unwrap_or(i64::MAX),
        );
        // Road-only readiness does not wait for terrain; full mode pins both products.
        // Preview readiness describes road geometry only. Terrain and full commit readiness
        // are resolved after the exact click under the simulation lock.
        let mut terrain_patches = None;
        dict.set("include_terrain", preview.include_terrain);
        let valid = dict.get("is_valid").is_some_and(|value| value.to::<bool>());
        dict.set("plan_state", if valid { "provisional" } else { "invalid" });
        if let Some(plan) = preview.edit_plan() {
            let Some(core) = self.try_lock_core() else {
                return Variant::nil();
            };
            let state = if preview.include_terrain {
                plan.status(&core)
            } else {
                plan.road_status(&core)
            };
            if preview.include_terrain && valid && state == "ready" {
                terrain_patches = plan.terrain().and_then(|terrain| terrain.preview_patches());
            }
            dict.set("plan_state", if valid { state } else { "invalid" });
            if valid && state == "invalid" {
                dict.set("is_valid", false);
                dict.set(
                    "invalid_reason",
                    if plan.overlaps_fields(&core) {
                        "field_overlap"
                    } else if plan.overlaps_cell_zoning(&core) {
                        "cell_overlap"
                    } else {
                        "road_plan_invalid"
                    },
                );
            }
        }
        let readiness_ms = export_start.map(|t| t.elapsed().as_secs_f64() * 1000.0);
        Self::append_road_preview_visual_mesh(&mut dict, &preview.visual_mesh);
        if let Some(scene) = &preview.junction_preview {
            let empty = BTreeSet::new();
            let export = |meshes| {
                SimCore::road_mesh_chunks_dict(
                    meshes,
                    &empty,
                    true,
                    preview.surface_generation,
                    scene.chunk_span_m,
                    scene.chunk_origin_x_m,
                    scene.chunk_origin_z_m,
                )
            };
            let mut replacement = export(&scene.planned);
            // Approach clips follow the moving junction bounds, so they travel with every pose.
            replacement.set(
                "approach_chunks",
                export(&scene.approach).get("chunks").unwrap(),
            );
            if scene.retained_revision != retained_revision || retained_revision == 0 {
                let retained = export(scene.retained.as_ref());
                replacement.set("retained_chunks", retained.get("chunks").unwrap());
            }
            replacement.set(
                "retained_revision",
                i64::try_from(scene.retained_revision).unwrap_or(i64::MAX),
            );
            replacement.set(
                "source_mesh_generation",
                i64::try_from(scene.source_mesh_generation).unwrap_or(i64::MAX),
            );
            replacement.set(
                "replacement_keys",
                PackedInt32Array::from_iter(
                    scene
                        .replacement_chunks
                        .iter()
                        .flat_map(|key| [key.0, key.1]),
                ),
            );
            if let Some(patches) = terrain_patches {
                let mut payloads = Array::<VarDictionary>::new();
                for patch in patches {
                    let revision = preview.terrain_revisions.get(&patch.key).copied();
                    let revision = revision.and_then(|r| i64::try_from(r).ok()).unwrap_or(0);
                    // The caller's display for this revision is identical: send only the metadata
                    // that its residency and generation checks need. O(held revisions) per patch.
                    let unchanged = revision > 0 && held_terrain.contains(&revision);
                    let mut data = if unchanged {
                        Self::terrain_patch_metadata_dict(&patch.patch)
                    } else if patch.input_road_loops == 0 {
                        Self::terrain_patch_dict(&patch.patch, &[])
                    } else {
                        Self::cached_refined_terrain_patch_dict(&patch, false, &[])
                    };
                    data.set("terrain_revision", revision);
                    data.set("unchanged", unchanged);
                    data.set(
                        "surface_generation",
                        i64::try_from(preview.surface_generation).unwrap_or(i64::MAX),
                    );
                    data.set("render_step_mm", i64::from(patch.key.render_step_mm));
                    data.set(
                        "terrain_requires_engineered_refinement",
                        patch.input_road_loops > 0,
                    );
                    payloads.push(&data);
                }
                let mut batch = VarDictionary::new();
                batch.set("patches", payloads);
                dict.set("terrain_preview", batch);
            }
            dict.set("junction_preview", replacement);
        }
        if let (Some(t), Some(start)) = (&preview.timing, export_start) {
            let mut metrics = VarDictionary::new();
            for (name, value) in [
                ("result_read_lock_ms", result_lock_ms),
                ("queue_ms", t.queue_ms),
                ("idle_ms", t.idle_ms),
                ("context_ms", t.context_ms),
                ("road_ms", t.road_ms),
                ("earthworks_ms", t.earthworks_ms),
                ("core_wait_ms", t.core_wait_ms),
                ("capture_ms", t.capture_ms),
                ("terrain_ms", t.terrain_ms),
                ("retained_ms", t.retained_ms),
                ("worker_ms", t.worker_ms),
                (
                    "worker_end_to_poll_ms",
                    start.duration_since(t.completed_at).as_secs_f64() * 1000.0,
                ),
                ("readiness_ms", readiness_ms.unwrap_or(0.0)),
                (
                    "packing_ms",
                    start.elapsed().as_secs_f64() * 1000.0 - readiness_ms.unwrap_or(0.0),
                ),
            ] {
                metrics.set(name, value);
            }
            dict.set("preview_timing", metrics);
        }
        dict.to_variant()
    }

    fn append_road_preview_visual_mesh(dict: &mut VarDictionary, mesh: &RoadPreviewVisualMesh) {
        dict.set(
            "surface_vertices",
            PackedVector3Array::from(mesh.vertices.as_slice()),
        );
        dict.set("surface_uvs", PackedVector2Array::from(mesh.uvs.as_slice()));
        dict.set(
            "surface_colors",
            PackedColorArray::from(mesh.colors.as_slice()),
        );
    }

    fn road_candidate_validation_to_variant(
        &self,
        validation: &RoadPreviewValidation,
        prepared_points: &[Vector3],
        fwd_lanes: i32,
        bkw_lanes: i32,
    ) -> Variant {
        let dict = self
            .road_candidate_dictionary_with_parcel_clearance(
                validation,
                prepared_points,
                fwd_lanes,
                bkw_lanes,
            )
            .unwrap_or_else(|| {
                let mut pending =
                    Self::road_candidate_validation_to_dictionary(validation, prepared_points);
                pending.set("is_valid", false);
                pending.set("is_pending", true);
                pending
            });
        dict.to_variant()
    }

    fn road_candidate_dictionary_with_parcel_clearance(
        &self,
        validation: &RoadPreviewValidation,
        prepared_points: &[Vector3],
        fwd_lanes: i32,
        bkw_lanes: i32,
    ) -> Option<VarDictionary> {
        let mut dict = Self::road_candidate_validation_to_dictionary(validation, prepared_points);
        if validation.is_valid {
            // Query the existing parcel chunk index locally. Never copy city-wide zoning into
            // a preview snapshot or wait on the simulation lock from the mouse-hover path.
            let core = self.try_lock_core()?;
            let lanes =
                fwd_lanes.clamp(0, i32::from(u8::MAX)) + bkw_lanes.clamp(0, i32::from(u8::MAX));
            let half_width = (lanes as f32 * crate::config::LANE_WIDTH).max(2.0) * 0.5
                + crate::config::SIDEWALK_WIDTH;
            let overlaps = core
                .zoning
                .parcel_ids_overlapping_road_corridor(prepared_points, half_width);
            if core
                .allocator
                .field_clearance
                .overlaps_road_corridor(prepared_points, half_width)
            {
                dict.set("is_valid", false);
                dict.set("invalid_reason", "field_overlap");
            }
            dict.set(
                "zoning_revision",
                i64::try_from(core.zoning.overlay_revision()).unwrap_or(i64::MAX),
            );
            dict.set("field_revision", core.agriculture.visual_revision() as i64);
            if let Some(&first) = overlaps.first() {
                dict.set("is_valid", false);
                dict.set("invalid_reason", "parcel_overlap");
                dict.set("overlapping_parcel_count", overlaps.len() as i64);
                dict.set(
                    "first_overlapping_parcel_id",
                    i64::try_from(first).unwrap_or(i64::MAX),
                );
            }
        }
        Some(dict)
    }

    fn road_candidate_validation_to_dictionary(
        validation: &RoadPreviewValidation,
        prepared_points: &[Vector3],
    ) -> VarDictionary {
        let mut dict = Self::road_preview_validation_to_dictionary(validation);
        dict.set(
            "prepared_points",
            PackedVector3Array::from_iter(prepared_points.iter().copied()),
        );
        let build_length_m = Self::road_build_length_m(prepared_points);
        dict.set("build_length_m", build_length_m);
        dict.set("build_cost", build_length_m * ROAD_BUILD_COST_PER_METER);
        dict
    }

    fn road_build_length_m(points: &[Vector3]) -> f64 {
        points
            .windows(2)
            .map(|pair| {
                let dx = f64::from(pair[1].x - pair[0].x);
                let dy = f64::from(pair[1].y - pair[0].y);
                let dz = f64::from(pair[1].z - pair[0].z);
                (dx * dx + dy * dy + dz * dz).sqrt()
            })
            .sum()
    }

    fn road_preview_validation_to_dictionary(validation: &RoadPreviewValidation) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set("is_valid", validation.is_valid);
        dict.set("invalid_reason", validation.invalid_reason);
        dict.set("max_grade", validation.max_grade);
        dict.set("allowed_grade", validation.allowed_grade);
        dict.set("offending_span_start_m", validation.offending_span_start_m);
        dict.set("offending_span_end_m", validation.offending_span_end_m);
        dict.set("offending_span_run_m", validation.offending_span_run_m);
        dict.set(
            "offending_span_height_delta_m",
            validation.offending_span_height_delta_m,
        );
        dict.set(
            "offending_span_start_height_m",
            validation.offending_span_start_height_m,
        );
        dict.set(
            "offending_span_end_height_m",
            validation.offending_span_end_height_m,
        );
        dict.set(
            "offending_span_start_terrain_height_m",
            validation.offending_span_start_terrain_height_m,
        );
        dict.set(
            "offending_span_end_terrain_height_m",
            validation.offending_span_end_terrain_height_m,
        );
        dict.set(
            "offending_span_start_support_delta_m",
            validation.offending_span_start_support_delta_m,
        );
        dict.set(
            "offending_span_end_support_delta_m",
            validation.offending_span_end_support_delta_m,
        );
        dict.set(
            "start_endpoint_snapped_node_id",
            validation.start_endpoint_snapped_node_id,
        );
        dict.set(
            "end_endpoint_snapped_node_id",
            validation.end_endpoint_snapped_node_id,
        );
        dict.set(
            "start_endpoint_height_m",
            validation.start_endpoint_height_m,
        );
        dict.set("end_endpoint_height_m", validation.end_endpoint_height_m);
        dict.set(
            "start_endpoint_terrain_height_m",
            validation.start_endpoint_terrain_height_m,
        );
        dict.set(
            "end_endpoint_terrain_height_m",
            validation.end_endpoint_terrain_height_m,
        );
        dict.set(
            "start_endpoint_support_delta_m",
            validation.start_endpoint_support_delta_m,
        );
        dict.set(
            "end_endpoint_support_delta_m",
            validation.end_endpoint_support_delta_m,
        );
        dict.set("clearance_m", validation.clearance_m);
        dict.set("required_clearance_m", validation.required_clearance_m);
        dict
    }
}
