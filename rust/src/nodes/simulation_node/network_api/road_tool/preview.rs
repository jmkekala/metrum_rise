// SPDX-License-Identifier: GPL-2.0-only

//! Preview road-tool Godot API methods.

use super::super::super::*;
use crate::nodes::sim::core::RoadCorridorClearance;
use crate::simulation::network::road_edit::plan_road_edit;

#[godot_api(secondary)]
impl SimulationNode {
    /// Returns cheap synchronous hover feedback for a road-tool candidate.
    ///
    /// Returns prepared points and validation; the road tool's display comes from
    /// [`Self::preview_road`].
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
        let road_debug = crate::debug::Channel::Road.enabled();
        let total_start = road_debug.then(Instant::now);
        let point_count = points.len();
        let (surface_generation, (prepared_points, validation)) = {
            let query = self.road_tool.view.read().unwrap();
            let validated = query.validate(&RoadPreviewRequest {
                surface_generation: query.surface_generation,
                points: points.to_vec(),
                fwd_lanes,
                bkw_lanes,
                snap_to_existing_roads,
            });
            (query.surface_generation, validated)
        };
        let validation = &validation;
        if road_debug {
            debug_log!(
                "road",
                "road_candidate_fast_validation points={} prepared_points={} fwd_lanes={} bkw_lanes={} valid={} reason={} max_grade={:.3} allowed_grade={:.3} endpoint_snap=({},{}) total_ms={:.3}",
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
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }

        let result = self.road_candidate_validation_to_variant(
            validation,
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

    /// Validates a road-tool candidate against the committed graph's local edit plan, including
    /// the `ROAD-54` junction and curve limits, with optional snapping.
    #[func]
    pub fn validate_road_candidate_for_commit_with_snap(
        &self,
        points: PackedVector3Array,
        fwd_lanes: i32,
        bkw_lanes: i32,
        snap_to_existing_roads: bool,
    ) -> Variant {
        let road_debug = crate::debug::Channel::Road.enabled();
        let total_start = road_debug.then(Instant::now);
        let point_count = points.len();
        let points = points.to_vec();
        let fwd_lanes_u8 = fwd_lanes.clamp(0, i32::from(u8::MAX)) as u8;
        let bkw_lanes_u8 = bkw_lanes.clamp(0, i32::from(u8::MAX)) as u8;
        let (prepared_points, validation, surface_generation) = {
            let query = self.road_tool.view.read().unwrap();
            let (prepared_input, validation, _) = plan_road_edit(
                &points,
                fwd_lanes_u8,
                bkw_lanes_u8,
                &query.terrain,
                &query.region_graph,
                &query.roadbed,
                snap_to_existing_roads,
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
        if road_debug {
            debug_log!(
                "road",
                "road_candidate_commit_validation points={} prepared_points={} fwd_lanes={} bkw_lanes={} valid={} reason={} max_grade={:.3} allowed_grade={:.3} endpoint_snap=({},{}) total_ms={:.3}",
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

    /// Logs the road tool's refusal of a clicked stroke on `events`, from the validation the tool
    /// showed: the reason with its numbers and location, then a `road_attempt` line with the
    /// exact points, lanes, snapping and road-tool generation for replay. Returns the attempt
    /// number. Called only on a refused click.
    #[func]
    pub fn log_road_tool_rejection(
        &self,
        points: PackedVector3Array,
        fwd_lanes: i32,
        bkw_lanes: i32,
        snap_to_existing_roads: bool,
        validation: VarDictionary,
    ) -> i64 {
        use crate::nodes::sim::core::{RejectedBy, RoadAttempt, RoadRejection};
        let int = |key: &str| {
            validation
                .get(key)
                .and_then(|value| value.try_to::<i64>().ok())
        };
        let reason = validation
            .get("invalid_reason")
            .map(|value| value.to_string())
            .filter(|reason| !reason.is_empty())
            .unwrap_or_else(|| "unknown".to_owned());
        let prepared = validation
            .get("prepared_points")
            .and_then(|value| value.try_to::<PackedVector3Array>().ok())
            .map(|points| points.to_vec())
            .unwrap_or_default();
        let numbers = Self::road_preview_validation_from_dictionary(&validation);
        let mut rejection = RoadRejection::from_validation(&reason, &numbers, &prepared);
        if let (Some(count), Some(first)) = (
            int("overlapping_parcel_count"),
            int("first_overlapping_parcel_id"),
        ) {
            rejection = rejection.with_note(format!("{count} parcels, first {first}"));
        }
        let points = points.to_vec();
        let attempt = rejection.report(
            RejectedBy::Tool,
            RoadAttempt {
                points: &points,
                fwd_lanes,
                bkw_lanes,
                snap: snap_to_existing_roads,
            },
            int("surface_generation").map_or(0, |generation| generation.max(0) as u64),
        );
        i64::try_from(attempt).unwrap_or(i64::MAX)
    }

    /// The synchronous road preview (`ROAD-68-E`): plans the stroke on the road tool's context,
    /// compiles the pieces of its dirty closure and returns the validation, the dependency
    /// fingerprint, the per-stage times and the render chunks the pieces change. A click on
    /// the same stroke submits this preview, and the commit installs its pieces while the
    /// fingerprint is current. Takes the context lock only, never the core lock.
    #[func]
    pub fn preview_road(
        &self,
        points: PackedVector3Array,
        fwd_lanes: i32,
        bkw_lanes: i32,
        snap_to_existing_roads: bool,
    ) -> VarDictionary {
        let (preview, span_m, origin, view_terrain) = {
            let mut context = self
                .road_tool
                .context
                .lock()
                .expect("road tool context lock poisoned");
            let surface_generation = context.view.surface_generation;
            let preview = Arc::new(context.preview_road(RoadPreviewRequest {
                surface_generation,
                points: points.to_vec(),
                fwd_lanes,
                bkw_lanes,
                snap_to_existing_roads,
            }));
            context.last_preview = Some(Arc::clone(&preview));
            let roadbed = &context.view.roadbed;
            let (origin_x, origin_z) = roadbed.chunk_origin_m();
            (
                preview,
                roadbed.chunk_span_m(),
                (origin_x, origin_z),
                Arc::clone(&context.view.terrain),
            )
        };
        let mut dict = Self::road_candidate_validation_to_dictionary(
            &preview.validation,
            &preview.plan.prepared().points,
        );
        let generation = |value: u64| i64::try_from(value).unwrap_or(i64::MAX);
        let fingerprint = &preview.fingerprint;
        dict.set(
            "surface_generation",
            generation(fingerprint.surface_generation),
        );
        dict.set(
            "roadbed_generation",
            generation(fingerprint.roadbed_generation),
        );
        dict.set(
            "terrain_source_generation",
            generation(fingerprint.terrain_source_generation),
        );
        dict.set(
            "terrain_visual_generation",
            generation(fingerprint.terrain_visual_generation),
        );
        dict.set("plan_ms", preview.times.plan_ms);
        dict.set("roadbed_ms", preview.times.roadbed_ms);
        dict.set("payload_ms", preview.times.payload_ms);
        dict.set("terrain_ms", preview.times.terrain_ms);
        dict.set("graded_ms", preview.times.graded_ms);
        if let Some(pieces) = &preview.pieces {
            let stats = pieces.graded.stats;
            dict.set("graded_adopted", stats.adopted as i64);
            dict.set("graded_baked", stats.baked as i64);
            dict.set("graded_pending", pieces.graded.pending as i64);
        }
        let packing_start = std::time::Instant::now();
        let mut chunks = PackedInt32Array::new();
        let mut road_chunks = VarArray::new();
        let mut terrain_patches = VarArray::new();
        let mut terrain_error = String::new();
        if let Some(pieces) = &preview.pieces {
            for &(x, z) in &pieces.changed_chunks {
                chunks.push(x);
                chunks.push(z);
            }
            if let Some(meshes) = &pieces.chunks {
                road_chunks = Self::preview_road_chunk_updates(&pieces.changed_chunks, meshes);
            }
            match &pieces.terrain {
                Ok(terrain) => {
                    terrain_patches = Self::preview_terrain_patch_payloads(&view_terrain, terrain);
                }
                Err(reason) => terrain_error.clone_from(reason),
            }
        }
        dict.set("terrain_patches", terrain_patches);
        dict.set("terrain_error", terrain_error.as_str());
        dict.set(
            "terrain_generation",
            generation(crate::nodes::sim::core::ROAD_PREVIEW_TERRAIN_GENERATION),
        );
        let has_road_chunks = preview
            .pieces
            .as_ref()
            .is_some_and(|pieces| pieces.chunks.is_some());
        dict.set("has_road_chunks", has_road_chunks);
        dict.set("road_chunks", road_chunks);
        dict.set("chunk_span_m", span_m);
        dict.set("chunk_origin_x_m", origin.0);
        dict.set("chunk_origin_z_m", origin.1);
        dict.set(
            "packing_ms",
            packing_start.elapsed().as_secs_f64() * 1000.0,
        );
        let failed = preview
            .pieces
            .as_ref()
            .map_or(&[][..], |pieces| pieces.roadbed.failed_nodes());
        dict.set(
            "failed_nodes",
            PackedInt64Array::from_iter(failed.iter().map(|&node| i64::from(node))),
        );
        dict.set("has_pieces", preview.pieces.is_some());
        dict.set("changed_chunks", chunks);
        dict
    }

    /// One frame of the road preview's graded terrain without a new pose (`TERRAIN-07-E`,
    /// `docs/terrain.md` T27 (d)): stages the next bounded share of the latest pose's tiles.
    /// Returns `{}` once the shown tiles are the pose's, else `graded_ms` (the frame's terrain
    /// work), `graded_adopted`, `graded_baked` and `graded_pending` (the pose's tiles still to
    /// stage), as `preview_road` reports them for a pose's frame. Takes the context lock only.
    #[func]
    pub fn advance_road_preview_terrain(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        let pose = self
            .road_tool
            .context
            .lock()
            .expect("road tool context lock poisoned")
            .catch_up_graded();
        if let Some(pose) = pose {
            dict.set("graded_ms", pose.frame_ms);
            dict.set("graded_adopted", pose.stats.adopted as i64);
            dict.set("graded_baked", pose.stats.baked as i64);
            dict.set("graded_pending", pose.pending as i64);
        }
        dict
    }

    // The preview's terrain patches in the format `poll_ready_terrain_patch_payloads` returns
    // them, shading masks baked as the payload workers bake them (in parallel on the shared
    // pool): a refined patch's entry, or the raw heightmap patch of one the edit leaves raw.
    // `surface_generation` is the provisional preview generation, never acknowledged.
    fn preview_terrain_patch_payloads(
        terrain: &TerrainSystem,
        preview: &crate::nodes::sim::core::RoadPreviewTerrain,
    ) -> VarArray {
        use crate::nodes::sim::core::ROAD_PREVIEW_TERRAIN_GENERATION;
        let payloads: Vec<TerrainPatchPayload> = crate::thread_pools::install(|| {
            preview
                .patches
                .par_iter()
                .filter_map(|patch| {
                    let (key, data) = match &patch.entry {
                        Some(entry) => (
                            TerrainPatchPayloadKey {
                                patch_x: entry.key.patch_x,
                                patch_z: entry.key.patch_z,
                                render_step_mm: entry.key.render_step_mm,
                            },
                            TerrainPatchPayloadData::Refined {
                                patch: Arc::clone(entry),
                            },
                        ),
                        None => {
                            let snapshot =
                                terrain.visual_patch_snapshot(patch.key.0, patch.key.1)?;
                            let height_bytes = Self::f32_bytes_vec(&snapshot.height_data);
                            (
                                TerrainPatchPayloadKey {
                                    patch_x: patch.key.0,
                                    patch_z: patch.key.1,
                                    render_step_mm: 0,
                                },
                                TerrainPatchPayloadData::Regular {
                                    patch: snapshot,
                                    height_bytes,
                                },
                            )
                        }
                    };
                    let mut payload = TerrainPatchPayload {
                        key,
                        request_id: 0,
                        surface_generation: ROAD_PREVIEW_TERRAIN_GENERATION,
                        data,
                        mask_bytes: Vec::new(),
                    };
                    payload.bake_shading_masks();
                    Some(payload)
                })
                .collect()
        });
        let mut patches = VarArray::new();
        for payload in &payloads {
            patches.push(&Self::terrain_patch_payload_dict(payload).to_variant());
        }
        patches
    }

    // The preview's changed chunks in the committed publication's update format: a chunk the
    // preview empties is `removed`.
    fn preview_road_chunk_updates(
        changed: &[crate::simulation::network::roadbed::SurfaceChunkKey],
        meshes: &std::collections::BTreeMap<
            crate::simulation::network::roadbed::SurfaceChunkKey,
            Arc<crate::simulation::network::render::NetworkMeshData>,
        >,
    ) -> VarArray {
        let mut updates = VarArray::new();
        for chunk in changed {
            let mut update = match meshes.get(chunk) {
                Some(mesh) => {
                    let mut update = SimCore::network_mesh_data_dict(mesh);
                    update.set("removed", false);
                    update
                }
                None => {
                    let mut removed = VarDictionary::new();
                    removed.set("removed", true);
                    removed
                }
            };
            update.set("chunk_x", chunk.0);
            update.set("chunk_z", chunk.1);
            updates.push(&update.to_variant());
        }
        updates
    }

    /// Returns the road-tool surface snapshot generation currently used for validation.
    #[func]
    pub fn get_road_tool_surface_generation(&self) -> i64 {
        let query = self.road_tool.view.read().unwrap();
        i64::try_from(query.surface_generation).unwrap_or(i64::MAX)
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
            let clearance = RoadCorridorClearance::evaluate(
                &*self.try_lock_core()?,
                prepared_points,
                fwd_lanes,
                bkw_lanes,
            );
            Self::apply_corridor_clearance(&mut dict, &clearance);
        }
        Some(dict)
    }

    fn apply_corridor_clearance(dict: &mut VarDictionary, clearance: &RoadCorridorClearance) {
        if clearance.field_overlap {
            dict.set("is_valid", false);
            dict.set("invalid_reason", "field_overlap");
        }
        dict.set(
            "zoning_revision",
            i64::try_from(clearance.zoning_revision).unwrap_or(i64::MAX),
        );
        dict.set("field_revision", clearance.field_revision as i64);
        if let Some(first) = clearance.first_overlapping_parcel {
            dict.set("is_valid", false);
            dict.set("invalid_reason", "parcel_overlap");
            dict.set(
                "overlapping_parcel_count",
                clearance.overlapping_parcels as i64,
            );
            dict.set(
                "first_overlapping_parcel_id",
                i64::try_from(first).unwrap_or(i64::MAX),
            );
        }
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

    // Inverse of `road_preview_validation_to_dictionary` for the numbers; the reason stays with
    // the caller because tool checks add reasons the validation does not carry.
    fn road_preview_validation_from_dictionary(dict: &VarDictionary) -> RoadPreviewValidation {
        let number = |key: &str| {
            dict.get(key)
                .and_then(|value| value.try_to::<f32>().ok())
                .unwrap_or(0.0)
        };
        let node = |key: &str| {
            dict.get(key)
                .and_then(|value| value.try_to::<i64>().ok())
                .map_or(-1, |id| i32::try_from(id).unwrap_or(-1))
        };
        RoadPreviewValidation {
            is_valid: false,
            invalid_reason: "",
            max_grade: number("max_grade"),
            allowed_grade: number("allowed_grade"),
            warning_reason: "",
            warning_limit: number("warning_limit"),
            offending_span_start_m: number("offending_span_start_m"),
            offending_span_end_m: number("offending_span_end_m"),
            offending_span_run_m: number("offending_span_run_m"),
            offending_span_height_delta_m: number("offending_span_height_delta_m"),
            offending_span_start_height_m: number("offending_span_start_height_m"),
            offending_span_end_height_m: number("offending_span_end_height_m"),
            offending_span_start_terrain_height_m: number("offending_span_start_terrain_height_m"),
            offending_span_end_terrain_height_m: number("offending_span_end_terrain_height_m"),
            offending_span_start_support_delta_m: number("offending_span_start_support_delta_m"),
            offending_span_end_support_delta_m: number("offending_span_end_support_delta_m"),
            start_endpoint_snapped_node_id: node("start_endpoint_snapped_node_id"),
            end_endpoint_snapped_node_id: node("end_endpoint_snapped_node_id"),
            start_endpoint_height_m: number("start_endpoint_height_m"),
            end_endpoint_height_m: number("end_endpoint_height_m"),
            start_endpoint_terrain_height_m: number("start_endpoint_terrain_height_m"),
            end_endpoint_terrain_height_m: number("end_endpoint_terrain_height_m"),
            start_endpoint_support_delta_m: number("start_endpoint_support_delta_m"),
            end_endpoint_support_delta_m: number("end_endpoint_support_delta_m"),
            clearance_m: number("clearance_m"),
            required_clearance_m: number("required_clearance_m"),
            geometry_measured: number("geometry_measured"),
            geometry_limit: number("geometry_limit"),
        }
    }

    fn road_preview_validation_to_dictionary(validation: &RoadPreviewValidation) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set("is_valid", validation.is_valid);
        dict.set("invalid_reason", validation.invalid_reason);
        dict.set("max_grade", validation.max_grade);
        dict.set("allowed_grade", validation.allowed_grade);
        dict.set("warning_reason", validation.warning_reason);
        dict.set("warning_limit", validation.warning_limit);
        dict.set("geometry_measured", validation.geometry_measured);
        dict.set("geometry_limit", validation.geometry_limit);
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
