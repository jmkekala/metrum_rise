// SPDX-License-Identifier: GPL-2.0-only

//! Preview road-tool Godot API methods.

use super::super::super::*;
use crate::nodes::sim::core::RoadCorridorClearance;
use crate::simulation::network::road_edit::{prepare_road_input_for_tool, validate_road_edit};

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
        let road_debug = crate::debug::Channel::Road.enabled();
        let total_start = road_debug.then(Instant::now);
        let point_count = points.len();
        let preview = {
            let query = self.road_tool_query_snapshot.read().unwrap();
            query.preview(&RoadPreviewRequest {
                include_terrain: false,
                request_id: 0,
                surface_generation: query.surface_generation,
                points: points.to_vec(),
                fwd_lanes,
                bkw_lanes,
                snap_to_existing_roads,
            })
        };
        let validation = &preview.validation;
        if road_debug {
            debug_log!(
                "road",
                "road_candidate_fast_validation points={} prepared_points={} fwd_lanes={} bkw_lanes={} valid={} reason={} max_grade={:.3} allowed_grade={:.3} endpoint_snap=({},{}) total_ms={:.3}",
                point_count,
                preview.prepared_points.len(),
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
            &preview.prepared_points,
            fwd_lanes,
            bkw_lanes,
        );
        let mut dict = result.to::<VarDictionary>();
        dict.set(
            "surface_generation",
            i64::try_from(preview.surface_generation).unwrap_or(i64::MAX),
        );
        dict.to_variant()
    }

    /// Builds display-only fallback geometry from an already-prepared road profile.
    ///
    /// Returns nil if the source terrain/road revision changed. `ROAD-68-A` builds no road
    /// geometry, so the arrays are empty until the roadbed supplies span pieces.
    #[func]
    pub fn build_road_preview_ribbon(
        &self,
        _prepared_points: PackedVector3Array,
        _fwd_lanes: i32,
        _bkw_lanes: i32,
        surface_generation: i64,
    ) -> Variant {
        let generation = self
            .road_tool_query_snapshot
            .read()
            .unwrap()
            .surface_generation;
        if i64::try_from(generation).unwrap_or(i64::MAX) != surface_generation {
            return Variant::nil();
        }
        let mut dict = VarDictionary::new();
        dict.set("surface_generation", surface_generation);
        Self::append_empty_road_preview_mesh(&mut dict);
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
            let query = self.road_tool_query_snapshot.read().unwrap();
            let prepared_input = prepare_road_input_for_tool(
                &points,
                &query.terrain,
                &query.region_graph,
                &query.roadbed,
                snap_to_existing_roads,
            );
            let (validation, _) = validate_road_edit(
                &prepared_input,
                fwd_lanes_u8,
                bkw_lanes_u8,
                &query.terrain,
                &query.region_graph,
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
    ///
    /// The stroke is validated synchronously and published at once through
    /// [`Self::get_preview_road_surface_result`]. `ROAD-68-A` carries no road or terrain pieces.
    #[func]
    pub fn request_preview_road_surface_with_options(
        &self,
        points: PackedVector3Array,
        fwd_lanes: i32,
        bkw_lanes: i32,
        snap_to_existing_roads: bool,
        include_terrain: bool,
    ) -> i64 {
        let request_id = self
            .road_preview_request_counter
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        let preview = {
            let query = self
                .road_tool_query_snapshot
                .read()
                .expect("road query snapshot lock poisoned");
            query.preview(&RoadPreviewRequest {
                include_terrain,
                request_id,
                surface_generation: query.surface_generation,
                points: points.to_vec(),
                fwd_lanes,
                bkw_lanes,
                snap_to_existing_roads,
            })
        };
        *self
            .road_preview_result
            .write()
            .expect("road preview result lock poisoned") = Some(preview);
        i64::try_from(request_id).unwrap_or(i64::MAX)
    }

    /// Latest preview request that has started, or zero. Requests complete when they are made,
    /// so this is also the latest published result.
    #[func]
    pub fn get_preview_road_surface_started_request_id(&self) -> i64 {
        let started = self
            .road_preview_result
            .read()
            .expect("road preview result lock poisoned")
            .as_ref()
            .map_or(0, |preview| preview.request_id);
        i64::try_from(started).unwrap_or(i64::MAX)
    }

    /// Returns the completed road-tool preview for `request_id`, or `null` when it was superseded
    /// or the simulation holds its lock for the corridor checks. The retained and terrain
    /// revisions are accepted for the road tool's protocol; `ROAD-68-A` exports no meshes.
    #[func]
    pub fn get_preview_road_surface_result(
        &self,
        request_id: i64,
        _retained_revision: i64,
        _terrain_revisions: PackedInt64Array,
    ) -> Variant {
        let Ok(request_id) = u64::try_from(request_id) else {
            return Variant::nil();
        };
        let preview_result = self
            .road_preview_result
            .read()
            .expect("road preview result lock poisoned");
        let Some(preview) = preview_result
            .as_ref()
            .filter(|preview| preview.request_id == request_id)
        else {
            return Variant::nil();
        };
        let Some(mut dict) = self.road_candidate_dictionary_with_parcel_clearance(
            &preview.validation,
            &preview.prepared_points,
            preview.fwd_lanes,
            preview.bkw_lanes,
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
        dict.set("include_terrain", preview.include_terrain);
        let valid = dict.get("is_valid").is_some_and(|value| value.to::<bool>());
        dict.set("plan_state", if valid { "provisional" } else { "invalid" });
        Self::append_empty_road_preview_mesh(&mut dict);
        dict.to_variant()
    }

    fn append_empty_road_preview_mesh(dict: &mut VarDictionary) {
        dict.set("surface_vertices", PackedVector3Array::new());
        dict.set("surface_uvs", PackedVector2Array::new());
        dict.set("surface_colors", PackedColorArray::new());
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
