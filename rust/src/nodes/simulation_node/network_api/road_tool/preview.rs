// SPDX-License-Identifier: GPL-2.0-only

//! Preview road-tool Godot API methods.

use super::super::super::*;
use crate::simulation::network::surface::RoadPreviewVisualMesh;

#[godot_api(secondary)]
impl SimulationNode {
    /// Returns cheap synchronous hover feedback for a road-tool candidate.
    ///
    /// Includes a terrain-draped display ribbon but does not compile junctions or mutate simulation
    /// state. The display positions must never be fed back into authoritative placement.
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
        let (prepared_points, validation, visual_mesh) = {
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
            let visual_mesh = query.road_surface.build_preview_visual_mesh(
                &prepared_input.points,
                &[],
                fwd_lanes.clamp(0, i32::from(u8::MAX)) as u8,
                bkw_lanes.clamp(0, i32::from(u8::MAX)) as u8,
                &query.terrain,
            );
            (prepared_input.points, validation, visual_mesh)
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
        Self::append_road_preview_visual_mesh(&mut dict, &visual_mesh);
        dict.to_variant()
    }

    /// Validates a road-tool candidate by compiling temporary surface geometry.
    ///
    /// This is intentionally not used by the interactive road-tool click path because complex
    /// junctions can take hundreds of milliseconds to compile. Keep it for diagnostics and tests
    /// that need to compare the fast placement contract against the full surface compiler.
    #[func]
    pub fn validate_road_candidate_for_commit(
        &self,
        points: PackedVector3Array,
        fwd_lanes: i32,
        bkw_lanes: i32,
    ) -> Variant {
        self.validate_road_candidate_for_commit_with_snap(points, fwd_lanes, bkw_lanes, true)
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
        let (prepared_points, validation) = {
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
            (prepared_input.points, validation)
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

        self.road_candidate_validation_to_variant(
            &validation,
            &prepared_points,
            fwd_lanes,
            bkw_lanes,
        )
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
        let send_ok = self
            .road_preview_tx
            .send(RoadPreviewRequest {
                request_id,
                surface_generation,
                points,
                fwd_lanes,
                bkw_lanes,
                snap_to_existing_roads,
            })
            .is_ok();
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

    /// Returns the completed road-tool preview for `request_id`, or `null` while pending/stale.
    #[func]
    pub fn get_preview_road_surface_result(&self, request_id: i64) -> Variant {
        let Ok(request_id) = u64::try_from(request_id) else {
            return Variant::nil();
        };
        let preview_result = self.road_preview_result.read().unwrap();
        let Some(preview) = preview_result.as_ref() else {
            return Variant::nil();
        };
        if preview.request_id != request_id {
            return Variant::nil();
        }

        self.road_preview_snapshot_to_variant(preview)
    }

    fn road_preview_snapshot_to_variant(&self, preview: &RoadPreviewSnapshot) -> Variant {
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
        Self::append_road_preview_visual_mesh(&mut dict, &preview.visual_mesh);
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
            dict.set(
                "zoning_revision",
                i64::try_from(core.zoning.overlay_revision()).unwrap_or(i64::MAX),
            );
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
