// SPDX-License-Identifier: GPL-2.0-only

//! Preview road-tool Godot API methods.

use super::super::super::*;
use crate::nodes::sim::core::RoadCorridorClearance;
use crate::nodes::sim::road_tool::height::{DraftSection, DraftStroke, draft_batch};
use crate::nodes::sim::road_tool::measure::RoadTypeSpec;
use crate::nodes::sim::road_tool::outcome::PreviewOutcome;
use crate::simulation::network::graph::{RoadMedian, RoadParking};
use crate::simulation::network::road_edit::{PreparedRoadInput, RoadProfile, plan_road_edit};
use crate::simulation::network::road_types::RoadAttributes;
use crate::simulation::network::road_types::cost as road_cost;
use crate::simulation::network::types::EdgeClass;

// A lane count from Godot, as the stroke's checks clamp it.
fn lanes_u8(count: i32) -> u8 {
    count.clamp(0, i32::from(u8::MAX)) as u8
}

// What a refusal report says of a clicked stroke (`report_road_tool_rejection`): the reason, the
// validation's numbers and the prepared points it locates them on, a corridor's overlapping
// parcels (their count and the first's id) and the road tool's surface generation.
pub(super) struct RejectionFacts<'a> {
    pub(super) reason: &'a str,
    pub(super) numbers: RoadPreviewValidation,
    pub(super) prepared: &'a [Vector3],
    pub(super) overlapping_parcels: Option<(i64, i64)>,
    pub(super) surface_generation: i64,
}

#[godot_api(secondary)]
impl SimulationNode {
    /// Sets the road tool's median (`ROAD-80`) for every later hover check, preview, commit and
    /// refusal report of the tool: `style` 0 none, 1 painted, 2 kerbed grass, 3 kerbed paving,
    /// `width_m` within the style's range. Returns the empty string when set, else the refusal
    /// key (`unknown_median_style`, `median_width_out_of_range`), keeping the previous median.
    /// Whether the stroke's lanes may carry it is the stroke's check
    /// (`median_requires_both_directions`, `median_requires_vehicle_road`).
    #[func]
    pub fn set_road_tool_median(&mut self, style: i32, width_m: f32) -> GString {
        match crate::simulation::network::graph::RoadMedian::from_tool(style, width_m) {
            Ok(median) => {
                self.set_road_tool_spec(RoadTypeSpec {
                    median,
                    ..self.road_tool_spec()
                });
                GString::new()
            }
            Err(reason) => reason.into(),
        }
    }

    /// Sets the road tool's kerbside parking (`PARK-05`) for every later hover check, preview,
    /// commit and refusal report of the tool: `code` bit 0 the left kerb, bit 1 the right, looking
    /// along the stroke. Returns the empty string when set, else `parking_code_out_of_range`,
    /// keeping the previous setting. Whether the stroke's lanes may carry it is the stroke's
    /// check (`parking_requires_vehicle_road`, `layout_parking_needs_sidewalk`).
    #[func]
    pub fn set_road_tool_parking(&mut self, code: i32) -> GString {
        match crate::simulation::network::graph::RoadParking::from_code(code) {
            Some(parking) => {
                self.set_road_tool_spec(RoadTypeSpec {
                    parking,
                    ..self.road_tool_spec()
                });
                GString::new()
            }
            None => "parking_code_out_of_range".into(),
        }
    }

    /// Sets the road tool's road-type attributes (`ROAD-101`) for every later hover check,
    /// preview, commit and refusal report of the tool: `speed_kmh` the speed limit (20 to 120 in
    /// steps of 10; a walkway's stroke keeps the default limit whatever is set), `sidewalks`
    /// (`ROAD-107`: the sides of a car road with one in the stroke's direction, 0 none, 1 left,
    /// 2 right, 3 both, a shoulder on a side without; a walkway is walked either way), `zoning`
    /// (lots and buildings along the road) and `street_lights` (visual only). Returns the empty
    /// string when set, else `speed_out_of_range` or `sidewalks_out_of_range`, keeping the
    /// previous attributes. The built-in types send (50, 3, true, true): the edge every stroke
    /// built before road types.
    #[func]
    pub fn set_road_tool_attributes(
        &mut self,
        speed_kmh: i32,
        sidewalks: i32,
        zoning: bool,
        street_lights: bool,
    ) -> GString {
        match RoadAttributes::from_tool(speed_kmh.into(), sidewalks.into(), zoning, street_lights) {
            Ok(attributes) => {
                self.set_road_tool_spec(RoadTypeSpec {
                    attributes,
                    ..self.road_tool_spec()
                });
                GString::new()
            }
            Err(reason) => reason.into(),
        }
    }

    /// Each median style's widths in metres (`ROAD-80`), keyed by the style code
    /// [`Self::set_road_tool_median`] takes (1 painted, 2 kerbed grass, 3 kerbed paving): a
    /// dictionary of `default_m` (the width a picked style starts at), `min_m` and `max_m` (the
    /// range outside which a width is refused). The table is constant; the road tool's keys and
    /// the cross-section controls read it once.
    #[func]
    pub fn get_median_styles(&self) -> VarDictionary {
        use crate::simulation::network::graph::{MedianStyle, RoadMedian};
        let mut styles = VarDictionary::new();
        for style in MedianStyle::ALL {
            let (min_m, max_m) = RoadMedian::width_range_m(style);
            let mut widths = VarDictionary::new();
            widths.set("default_m", RoadMedian::default_width_m(style));
            widths.set("min_m", min_m);
            widths.set("max_m", max_m);
            styles.set(style as i64, widths.to_variant());
        }
        styles
    }

    /// Checks a road-tool stroke once: the hover checks (profile, endpoints, water) on the road
    /// tool's published view, then a valid stroke's corridor against the live parcels and field
    /// reservations, waiting for the core for that (`ROAD-102`). Returns the prepared points and
    /// the validation; a stroke that is not valid always names its `invalid_reason`. The road
    /// tool's own hover is [`Self::preview_road`], which takes no core.
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
        let spec = self.road_tool_spec();
        let point_count = points.len();
        let (surface_generation, (prepared, validation)) = {
            let query = self.road_tool.view.read().unwrap();
            let validated = query.validate(&RoadPreviewRequest {
                surface_generation: query.surface_generation,
                points: points.to_vec(),
                fwd_lanes,
                bkw_lanes,
                snap_to_existing_roads,
                profile: RoadProfile::FollowGround,
                median: spec.median,
                parking: spec.parking,
                attributes: spec.attributes,
            });
            (query.surface_generation, validated)
        };
        let validation = &validation;
        let prepared_points = &prepared.points;
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
            prepared_points,
            (fwd_lanes, bkw_lanes),
            prepared.class,
            RoadPreviewRequest {
                surface_generation,
                points: points.to_vec(),
                fwd_lanes,
                bkw_lanes,
                snap_to_existing_roads,
                profile: RoadProfile::FollowGround,
                median: spec.median,
                parking: spec.parking,
                attributes: spec.attributes,
            },
        );
        let mut dict = result.to::<VarDictionary>();
        dict.set(
            "surface_generation",
            i64::try_from(surface_generation).unwrap_or(i64::MAX),
        );
        dict.to_variant()
    }

    /// Validates a road-tool candidate against the committed graph's local edit plan, including
    /// the `ROAD-54` junction and curve limits, with optional snapping; a valid stroke's corridor
    /// clearance waits for the core as [`Self::validate_road_candidate_with_snap`]'s does.
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
        let spec = self.road_tool_spec();
        let (prepared_input, validation, surface_generation) = self.plan_commit_check(
            &points,
            (fwd_lanes, bkw_lanes),
            spec.stroke(),
            snap_to_existing_roads,
        );
        let class = prepared_input.class;
        let prepared_points = prepared_input.points;
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
                (fwd_lanes, bkw_lanes),
                class,
                RoadPreviewRequest {
                    surface_generation,
                    points,
                    fwd_lanes,
                    bkw_lanes,
                    snap_to_existing_roads,
                    profile: RoadProfile::FollowGround,
                    median: spec.median,
                    parking: spec.parking,
                    attributes: spec.attributes,
                },
            )
            .to::<VarDictionary>();
        dict.set(
            "surface_generation",
            i64::try_from(surface_generation).unwrap_or(i64::MAX),
        );
        dict.to_variant()
    }

    /// Replays one refused stroke's `road_attempt` record (the debug console's replay,
    /// `road_attempt_replay.gd`) with the record's own lanes, snapping, profile, median, parking
    /// and road-type attributes; the road tool's intent stays as it is (`ROAD-112` round R2-6).
    /// A follow-ground stroke takes the tool's click check first, which waits for the core for
    /// its corridor clearance, so it always answers (`ROAD-102`); a stroke of a segmented draft
    /// (held ground, bridge or tunnel) goes to the commit alone, since the click check knows only
    /// the follow-ground profile. A field an older record lacks replays its default: one lane
    /// each way, snapping on, the tunnel flag of `ROAD-61` for the profile, no median, no
    /// parking, the built-ins' 50 km/h road with sidewalks on both sides, zoning and street
    /// lights. Returns `reason`, the key that refused the stroke before the commit (`too_short`
    /// under two points, the section's own refusal, `road_profile_unknown`, else the click
    /// check's), empty when queued, and `request_id`, the commit to poll with
    /// `get_road_commit_result` (0 when refused before).
    #[func]
    pub fn replay_road_attempt(&mut self, attempt: VarDictionary) -> VarDictionary {
        let (reason, request_id) = match self.replay_attempt(&attempt) {
            Ok(request_id) => ("", request_id),
            Err(reason) => (reason, 0),
        };
        vdict! { "reason": reason, "request_id": request_id }
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
        self.report_road_tool_rejection(
            RejectionFacts {
                reason: &reason,
                numbers: Self::road_preview_validation_from_dictionary(&validation),
                prepared: &prepared,
                overlapping_parcels: int("overlapping_parcel_count")
                    .zip(int("first_overlapping_parcel_id")),
                surface_generation: int("surface_generation").unwrap_or(0),
            },
            &points.to_vec(),
            [fwd_lanes, bkw_lanes],
            snap_to_existing_roads,
            RoadProfile::FollowGround,
        )
    }

    // [`Self::log_road_tool_rejection`] from typed facts (the road tool's standing outcome,
    // `ROAD-112` round R2-1), for a stroke of `profile` (a graded draft's stroke, round I).
    pub(super) fn report_road_tool_rejection(
        &self,
        facts: RejectionFacts<'_>,
        points: &[Vector3],
        lanes: [i32; 2],
        snap_to_existing_roads: bool,
        profile: RoadProfile,
    ) -> i64 {
        use crate::nodes::sim::core::{RejectedBy, RoadAttempt, RoadRejection};
        let [fwd_lanes, bkw_lanes] = lanes;
        let mut rejection =
            RoadRejection::from_validation(facts.reason, &facts.numbers, facts.prepared);
        if let Some((count, first)) = facts.overlapping_parcels {
            rejection = rejection.with_note(format!("{count} parcels, first {first}"));
        }
        let spec = self.road_tool_spec();
        let attempt = rejection.report(
            RejectedBy::Tool,
            RoadAttempt {
                points,
                fwd_lanes,
                bkw_lanes,
                snap: snap_to_existing_roads,
                profile,
                median: spec.median,
                parking: spec.parking,
                attributes: spec.attributes,
            },
            facts.surface_generation.max(0) as u64,
        );
        i64::try_from(attempt).unwrap_or(i64::MAX)
    }

    /// The synchronous road preview (`ROAD-68-E`): plans the stroke on the road tool's context,
    /// compiles the pieces of its dirty closure and returns the validation, the dependency
    /// fingerprint, the per-stage times and, for each render chunk the pieces change, the
    /// rebuilt pieces' triangles and the published runs they replace (`road_chunks`, below the
    /// chunk since `ROAD-68-H` round 3), indexing the resident chunks of `road_mesh_generation`.
    /// A click on the same stroke submits this preview, and the commit installs its pieces while
    /// the fingerprint is current. `graded_cut` (flat `(x, z)`, under `graded_epoch`): the tiles
    /// whose installed graded tile draws its road cut as the backstop while it shows under this
    /// pose (T65 option (c), `ROAD-112` round G2; empty when the pose draws no road). Takes the
    /// context lock only, never the core lock.
    #[func]
    pub fn preview_road(
        &self,
        points: PackedVector3Array,
        fwd_lanes: i32,
        bkw_lanes: i32,
        snap_to_existing_roads: bool,
    ) -> VarDictionary {
        let mut dict = self.preview_stroke(
            points.as_slice(),
            [fwd_lanes, bkw_lanes],
            snap_to_existing_roads,
            RoadProfile::FollowGround,
            None,
            &mut PreviewOutcome::default(),
        );
        dict.set("road_chunks", self.road_tool_last_preview_chunks());
        dict
    }

    /// The last single-stroke preview's `road_chunks` (tests that read a payload): the
    /// chunks' overlays in the committed publication's mesh format, with their hidden runs,
    /// as `preview_road` returns them; empty without chunks. Built on demand: the road tool's
    /// own previews hand their chunks to the draw module, not to the script (round R2-4a).
    #[func]
    pub fn road_tool_preview_chunks(&self) -> VarArray {
        self.road_tool_last_preview_chunks()
    }

    // [`Self::preview_road`] for a stroke already in Rust (the road tool session's), with its
    // `profile`; with `hue`, the stroke's footprint hue built with the preview
    // (`RoadPreviewContext::preview_road_with_hue`, `ROAD-123` round HF2). Writes the typed
    // outcome into `outcome` (round R2-1: its prepared-points buffer reused) and returns the
    // dictionary, both from the same preview.
    pub(super) fn preview_stroke(
        &self,
        points: &[Vector3],
        lanes: [i32; 2],
        snap_to_existing_roads: bool,
        profile: RoadProfile,
        hue: Option<(&mut crate::nodes::sim::road_tool::hue::OffFrameHue, f32)>,
        outcome: &mut PreviewOutcome,
    ) -> VarDictionary {
        let [fwd_lanes, bkw_lanes] = lanes;
        let mut graded_tiles = Vec::new();
        let mut graded_removed = Vec::new();
        let mut graded_cut = Vec::new();
        let spec = self.road_tool_spec();
        let land = self.road_tool.land();
        let (
            preview,
            span_m,
            origin,
            view_terrain,
            graded_epoch,
            road_mesh_generation,
            demolish_lots,
        ) = {
            let mut context = self
                .road_tool
                .context
                .lock()
                .expect("road tool context lock poisoned");
            context.land = land;
            let surface_generation = context.view.surface_generation;
            let preview = Arc::new(context.preview_road_with_hue(RoadPreviewRequest {
                surface_generation,
                points: points.to_vec(),
                fwd_lanes,
                bkw_lanes,
                snap_to_existing_roads,
                profile,
                median: spec.median,
                parking: spec.parking,
                attributes: spec.attributes,
            }, hue));
            context.last_preview = Some(Arc::clone(&preview));
            context
                .graded
                .take_shown_changes(&mut graded_tiles, &mut graded_removed);
            // T65 option (c): the tiles whose published cut this pose's road may leave open,
            // none when it draws no road.
            {
                let context = &mut *context;
                let delta = (preview.pieces.as_ref())
                    .filter(|_| preview.validation.is_valid)
                    .map(|pieces| &pieces.delta);
                graded_cut.extend(
                    (context.graded.cut_tiles(&context.view.roadbed, delta).iter())
                        .flat_map(|tile| [tile.x, tile.z]),
                );
            }
            let demolish_lots = preview
                .pieces
                .as_ref()
                .map(|pieces| Self::demolish_lot_outlines(&context, &pieces.take))
                .unwrap_or_default();
            let roadbed = &context.view.roadbed;
            let (origin_x, origin_z) = roadbed.chunk_origin_m();
            (
                preview,
                roadbed.chunk_span_m(),
                (origin_x, origin_z),
                Arc::clone(&context.view.terrain),
                context.graded.epoch(),
                context.view.road_mesh_surface_generation,
                demolish_lots,
            )
        };
        let prepared = preview.plan.prepared();
        let weight = Self::stroke_cost_weight((fwd_lanes, bkw_lanes), prepared);
        let mut dict = Self::road_candidate_validation_to_dictionary(
            &preview.validation,
            &prepared.points,
            weight,
        );
        let generation = |value: u64| i64::try_from(value).unwrap_or(i64::MAX);
        let fingerprint = &preview.fingerprint;
        let demolish_count = preview
            .pieces
            .as_ref()
            .map_or(0, |pieces| pieces.take.buildings.len() as i64);
        let validation = &preview.validation;
        let (build_length_m, build_cost) = Self::stroke_build(&prepared.points, weight);
        outcome.valid = validation.is_valid;
        outcome.pending = false;
        outcome.reason = validation.invalid_reason.into();
        outcome.draft_stroke = None;
        outcome.build_cost = build_cost;
        outcome.length_m = build_length_m;
        outcome.surface_generation = generation(fingerprint.surface_generation);
        outcome.validation = Some(validation.clone());
        outcome.prepared.clear();
        outcome.prepared.extend_from_slice(&prepared.points);
        outcome.demolish_count = demolish_count;
        outcome.marks.clone_from(&preview.marks);
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
        dict.set("tunnel", preview.plan.profile() == RoadProfile::Tunnel);
        dict.set("profile", i64::from(preview.plan.profile().code()));
        dict.set("plan_ms", preview.times.plan_ms);
        dict.set("roadbed_ms", preview.times.roadbed_ms);
        dict.set("payload_ms", preview.times.payload_ms);
        dict.set("graded_ms", preview.times.graded_ms);
        if let Some(pieces) = &preview.pieces {
            let stats = pieces.graded.stats;
            dict.set("graded_adopted", stats.adopted as i64);
            dict.set("graded_baked", stats.baked as i64);
            dict.set("graded_pending", pieces.graded.pending as i64);
        }
        let packing_start = std::time::Instant::now();
        let mut chunks = PackedInt32Array::new();
        outcome.overlay = None;
        if let Some(pieces) = &preview.pieces {
            for &(x, z) in &pieces.changed_chunks {
                chunks.push(x);
                chunks.push(z);
            }
            outcome.overlay = pieces.overlay.clone();
        }
        let cell_m = f64::from(view_terrain.cell_size_m());
        Self::set_graded_preview_changes(
            &mut dict,
            &graded_tiles,
            &graded_removed,
            graded_epoch,
            cell_m,
        );
        // A pose's frame replaces its epoch's backstop tiles (flat `(x, z)`); a catch-up frame
        // carries none and keeps them.
        dict.set("graded_cut", PackedInt32Array::from(graded_cut.as_slice()));
        let has_road_chunks = preview
            .pieces
            .as_ref()
            .is_some_and(|pieces| pieces.chunks.is_some());
        dict.set("has_road_chunks", has_road_chunks);
        dict.set("road_mesh_generation", generation(road_mesh_generation));
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
        // What the stroke takes (`ZONE-09`): the buildings it demolishes and the outlines of
        // the lots it removes, four corners each.
        dict.set("demolish_count", demolish_count);
        dict.set("demolish_lots", demolish_lots);
        dict
    }

    // The outlines of the lots a preview's take removes, on the view's graded ground
    // ([`lot_outlines`]). A lot the land does not hold (published a frame apart from the cells)
    // has none.
    fn demolish_lot_outlines(
        context: &crate::nodes::sim::core::RoadPreviewContext,
        take: &crate::nodes::sim::core::RoadLandTake<u64>,
    ) -> PackedVector3Array {
        view_lot_outlines(context, take)
    }

    /// One frame of the road preview's graded terrain without a new pose (`TERRAIN-07-E`,
    /// `docs/terrain.md` T27 (d)): stages the next bounded share of the latest pose's tiles.
    /// Returns `{}` once the shown tiles are the pose's, else `graded_ms` (the frame's terrain
    /// work), `graded_adopted`, `graded_baked` and `graded_pending` (the pose's tiles still to
    /// stage), as `preview_road` reports them for a pose's frame, with the shown tiles' changes
    /// as `preview_road` returns them. Takes the context lock only.
    #[func]
    pub fn advance_road_preview_terrain(&self) -> VarDictionary {
        self.road_preview_terrain_step().unwrap_or_default()
    }

    // `advance_road_preview_terrain`'s step, `None` (no allocation) once the shown tiles are the
    // pose's: the road tool's tick stages through it (round R2-2).
    pub(super) fn road_preview_terrain_step(&self) -> Option<VarDictionary> {
        let mut tiles = Vec::new();
        let mut removed = Vec::new();
        let (pose, epoch, cell_m) = {
            let mut context = self
                .road_tool
                .context
                .lock()
                .expect("road tool context lock poisoned");
            let pose = context.catch_up_graded();
            if pose.is_some() {
                context.graded.take_shown_changes(&mut tiles, &mut removed);
            }
            let cell_m = f64::from(context.view.terrain.cell_size_m());
            (pose, context.graded.epoch(), cell_m)
        };
        let pose = pose?;
        let mut dict = VarDictionary::new();
        dict.set("graded_ms", pose.frame_ms);
        dict.set("graded_adopted", pose.stats.adopted as i64);
        dict.set("graded_baked", pose.stats.baked as i64);
        dict.set("graded_pending", pose.pending as i64);
        Self::set_graded_preview_changes(&mut dict, &tiles, &removed, epoch, cell_m);
        Some(dict)
    }

    /// The graded terrain renderer's view (`TERRAIN-07-E`, T42): `camera` (XZ, with its height
    /// above the ground as `y`), where `GL0`-`GL2` end (`level_end_m`) and where `GL3` at steps
    /// 2, 4 and 8 ends (`gl3_end_m`, the last infinite). The road preview's next poses stage
    /// each tile at the level this distance rule gives it, without the hysteresis the renderer
    /// keeps around a held tile's level; a commit stages at its preview's levels. The same
    /// camera orders the pending tile bakes, nearest first (`TERRAIN-13` part 1). Called when
    /// the camera moves. Takes the context lock and the graded jobs' lock.
    #[func]
    pub fn set_road_preview_terrain_view(
        &self,
        camera: Vector3,
        level_end_m: PackedFloat64Array,
        gl3_end_m: PackedFloat64Array,
    ) {
        self.graded_jobs
            .lock()
            .unwrap()
            .set_view([camera.x, camera.y, camera.z].map(f64::from));
        let (Ok(level_end_m), Ok(gl3_end_m)) = (
            <[f64; 3]>::try_from(level_end_m.as_slice()),
            <[f64; 3]>::try_from(gl3_end_m.as_slice()),
        ) else {
            return;
        };
        let rule = crate::simulation::terrain::graded::LevelRule {
            view: Some(crate::simulation::terrain::graded::LevelView {
                camera: [camera.x, camera.y, camera.z].map(f64::from),
                level_end_m,
                gl3_end_m,
            }),
        };
        self.road_tool
            .context
            .lock()
            .expect("road tool context lock poisoned")
            .graded
            .set_rule(rule);
    }

    /// Ends the road preview's graded tiles (`TERRAIN-07-E`): at a click, whose commit installs
    /// the tiles of the returned epoch and releases the renderer's preview tiles of it, or at
    /// cancel, after which the renderer drops them itself. The next poses send their tiles
    /// under the next epoch. Takes the context lock only.
    #[func]
    pub fn release_road_preview_terrain(&self) -> i64 {
        let epoch = self
            .road_tool
            .context
            .lock()
            .expect("road tool context lock poisoned")
            .graded
            .release();
        i64::try_from(epoch).unwrap_or(i64::MAX)
    }

    // The shown graded tiles' changes of a preview frame: `graded_tiles` (packed as
    // `poll_graded_tile_results` packs its tiles, the token the epoch), `graded_removed` (flat
    // `(x, z)` pairs of the tiles shown no more) and `graded_epoch`.
    pub(super) fn set_graded_preview_changes(
        dict: &mut VarDictionary,
        tiles: &[crate::simulation::terrain::graded::StagedTile],
        removed: &[crate::simulation::terrain::graded::TileKey],
        epoch: u64,
        cell_m: f64,
    ) {
        let epoch = i64::try_from(epoch).unwrap_or(i64::MAX);
        let pack_start = std::time::Instant::now();
        dict.set(
            "graded_tiles",
            super::super::super::graded_tiles_api::packed_tile_dicts(tiles, epoch, cell_m),
        );
        let flat: Vec<i32> = removed.iter().flat_map(|tile| [tile.x, tile.z]).collect();
        dict.set("graded_removed", PackedInt32Array::from(flat.as_slice()));
        dict.set("graded_epoch", epoch);
        dict.set(
            "graded_pack_ms",
            pack_start.elapsed().as_secs_f64() * 1000.0,
        );
    }

    // `road_tool_preview_chunks`: the context's last preview's chunks as `road_chunks`.
    fn road_tool_last_preview_chunks(&self) -> VarArray {
        let context = self
            .road_tool
            .context
            .lock()
            .expect("road tool context lock poisoned");
        (context.last_preview.as_ref())
            .and_then(|preview| preview.pieces.as_ref())
            .and_then(|pieces| pieces.chunks.as_ref())
            .map(Self::preview_road_chunk_updates)
            .unwrap_or_default()
    }

    // The preview's chunks below the chunk (`ROAD-68-H` round 3): each overlay in the committed
    // publication's mesh format, with `hidden` (flat `[layer, start, end]` vertex ranges of the
    // resident chunk's runs it replaces) and `hidden_positions` (their published positions).
    pub(super) fn preview_road_chunk_updates(
        chunks: &std::collections::BTreeMap<
            crate::simulation::network::roadbed::SurfaceChunkKey,
            crate::simulation::network::render::road::PreviewChunk,
        >,
    ) -> VarArray {
        let mut updates = VarArray::new();
        for (chunk, preview) in chunks {
            let mut update = SimCore::network_mesh_data_dict(&preview.overlay);
            let hidden: Vec<i32> = preview
                .hidden
                .iter()
                .flatten()
                .map(|&value| value as i32)
                .collect();
            update.set("hidden", PackedInt32Array::from(hidden.as_slice()));
            update.set(
                "hidden_positions",
                PackedVector3Array::from(preview.hidden_positions.as_slice()),
            );
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

    // The validation as a dictionary; a valid stroke's corridor and pieces are checked once,
    // waiting for the core (`ROAD-102`). `request` is the stroke as the road tool would preview
    // it.
    fn road_candidate_validation_to_variant(
        &self,
        validation: &RoadPreviewValidation,
        prepared_points: &[Vector3],
        lanes: (i32, i32),
        class: EdgeClass,
        request: RoadPreviewRequest,
    ) -> Variant {
        let (fwd_lanes, bkw_lanes) = lanes;
        let weight = road_cost::stroke_weight(
            (lanes_u8(fwd_lanes), lanes_u8(bkw_lanes)),
            request.median,
            request.parking,
            request.attributes,
            class,
        );
        let mut dict =
            Self::road_candidate_validation_to_dictionary(validation, prepared_points, weight);
        if validation.is_valid {
            let clearance = self.corridor_clearance(prepared_points, lanes, request);
            Self::apply_corridor_clearance(&mut dict, &clearance);
        }
        dict.to_variant()
    }

    // A valid stroke's corridor against the live parcels and field reservations, with the
    // section `request` carries.
    fn corridor_clearance(
        &self,
        prepared_points: &[Vector3],
        lanes: (i32, i32),
        request: RoadPreviewRequest,
    ) -> RoadCorridorClearance {
        {
            // Query the existing parcel chunk index locally; never copy city-wide zoning into a
            // preview snapshot. Every caller asks once (tests, the road benchmark's fixture
            // search, the debug replay), so the check waits for the core: with `try_lock` it
            // answered "pending" (not valid, no reason) while the simulation held the core, which
            // its callers read as a refusal without a reason (`ROAD-102`).
            // Plotted land is judged on the stroke's pieces, compiled on the road tool's context
            // as its preview compiles them, so a bridge's deck refuses nothing its commit
            // accepts (`ZONE-09` decisions D2, D8). The context is locked before the core, as
            // everywhere (the simulation publishes to it without the core).
            let stroke = (request.median, request.parking, request.attributes);
            let evaluate = |pieces: Option<crate::nodes::sim::core::StrokePieces<'_>>| {
                RoadCorridorClearance::evaluate(
                    &self.lock_core(),
                    prepared_points,
                    lanes,
                    stroke,
                    pieces,
                )
            };
            let mut context = self
                .road_tool
                .context
                .lock()
                .expect("road tool context lock poisoned");
            let plan = crate::nodes::sim::core::RoadEditPlan::compile_from(
                &context.view.terrain,
                &context.replica,
                &context.view.roadbed,
                &*context.land,
                request,
            );
            let clearance = context
                .with_plan_pieces(&plan, |pieces| evaluate(Some(pieces)))
                .unwrap_or_else(|| evaluate(None));
            drop(context);
            clearance
        }
    }

    // Plotted reservations refuse the stroke; parcels are reported only, since the commit
    // takes zoned land (`ZONE-09`, decision D8).
    fn apply_corridor_clearance(dict: &mut VarDictionary, clearance: &RoadCorridorClearance) {
        if let Some(reason) = Self::corridor_refusal(clearance) {
            dict.set("is_valid", false);
            dict.set("invalid_reason", reason);
        }
        dict.set(
            "zoning_revision",
            i64::try_from(clearance.zoning_revision).unwrap_or(i64::MAX),
        );
        dict.set("field_revision", clearance.field_revision as i64);
        if let Some(first) = clearance.first_overlapping_parcel {
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

    // The reservation a stroke's corridor overlaps, as its refusal key.
    fn corridor_refusal(clearance: &RoadCorridorClearance) -> Option<&'static str> {
        [
            (clearance.field_overlap, "field_overlap"),
            (clearance.extraction_overlap, "extraction_area_overlap"),
            (clearance.plotted_overlap, "plotted_building_overlap"),
        ]
        .into_iter()
        .find_map(|(overlap, reason)| overlap.then_some(reason))
    }

    // The commit check of `points` with `lanes` and `stroke`'s median, parking and attributes on
    // the road tool's published view: the local edit plan's validation, then water. Returns the
    // prepared input, the validation and the view's surface generation.
    fn plan_commit_check(
        &self,
        points: &[Vector3],
        lanes: (i32, i32),
        stroke: (Option<RoadMedian>, RoadParking, RoadAttributes),
        snap_to_existing_roads: bool,
    ) -> (PreparedRoadInput, RoadPreviewValidation, u64) {
        let (fwd_lanes, bkw_lanes) = (lanes_u8(lanes.0), lanes_u8(lanes.1));
        let query = self.road_tool.view.read().unwrap();
        let land = self.road_tool.land();
        let (prepared_input, validation, _) = plan_road_edit(
            points,
            fwd_lanes,
            bkw_lanes,
            stroke,
            &query.terrain,
            &query.region_graph,
            &query.roadbed,
            &*land,
            snap_to_existing_roads,
            RoadProfile::FollowGround,
        );
        let validation = crate::nodes::sim::road_tool::validate_road_candidate_against_water(
            &prepared_input,
            fwd_lanes,
            bkw_lanes,
            &query.water,
            validation,
        );
        (prepared_input, validation, query.surface_generation)
    }

    // The validation with the prepared points, the stroke's build length and its cost at the
    // road type's `weight` (`ROAD-101`).
    fn road_candidate_validation_to_dictionary(
        validation: &RoadPreviewValidation,
        prepared_points: &[Vector3],
        weight: f64,
    ) -> VarDictionary {
        let mut dict = Self::road_preview_validation_to_dictionary(validation);
        dict.set(
            "prepared_points",
            PackedVector3Array::from_iter(prepared_points.iter().copied()),
        );
        let (build_length_m, build_cost) = Self::stroke_build(prepared_points, weight);
        dict.set("build_length_m", build_length_m);
        dict.set("build_cost", build_cost);
        dict
    }

    // A prepared stroke's build length and its cost at the road type's `weight`.
    fn stroke_build(prepared_points: &[Vector3], weight: f64) -> (f64, f64) {
        let build_length_m = Self::road_build_length_m(prepared_points);
        (build_length_m, build_length_m * road_cost::build_per_m(weight))
    }

    // A previewed stroke's cost weight: its lanes, and the median, parking, attributes and class
    // its prepared input carries.
    pub(super) fn stroke_cost_weight(lanes: (i32, i32), prepared: &PreparedRoadInput) -> f64 {
        road_cost::stroke_weight(
            (lanes_u8(lanes.0), lanes_u8(lanes.1)),
            prepared.median,
            prepared.parking,
            prepared.attributes,
            prepared.class,
        )
    }

    pub(super) fn road_build_length_m(points: &[Vector3]) -> f64 {
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
        let (junction_place, endpoint_edges) =
            crate::nodes::road_wording::read_refusal_places(dict);
        RoadPreviewValidation {
            junction_place,
            endpoint_edges,
            overlap_end: None,
            joined_nodes: [None; 2],
            contacts: [None; 2],
            lead_ins: [None; 2],
            growths: [None; 2],
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
        crate::nodes::road_wording::write_refusal_places(validation, &mut dict);
        dict
    }
}

// The outlines of the lots a take removes (`ZONE-09`, `ZONE-10`), one packer for every verdict
// that shows them: four corners per lot, front-left, front-right, rear-right, rear-left, on
// `surface`'s graded ground a little above it, as the bulldoze highlight lies. `corners` finds
// a lot by its id and a point inside it; a lot it does not find has none. O(lots * ground
// queries).
fn lot_outlines<B>(
    surface: &crate::simulation::terrain::graded::GradedSurface<'_>,
    scratch: &mut crate::simulation::terrain::graded::QueryScratch,
    take: &crate::nodes::sim::core::RoadLandTake<B>,
    corners: impl Fn(u64, glam::DVec2) -> Option<[Vector2; 4]>,
) -> PackedVector3Array {
    let mut outlines = PackedVector3Array::new();
    for (&lot, &at) in take.lots.iter().zip(&take.lot_points) {
        let Some(corners) = corners(lot, at) else {
            continue;
        };
        for corner in corners {
            let ground = surface.ground_height_at(f64::from(corner.x), f64::from(corner.y), scratch);
            outlines.push(Vector3::new(
                corner.x,
                ground as f32 + crate::nodes::sim::editing::BULLDOZE_HIGHLIGHT_Y_OFFSET_M,
                corner.y,
            ));
        }
    }
    outlines
}

/// [`lot_outlines`] on the road tool's context: the view's graded ground and the published land's
/// parcels. No core lock.
pub(crate) fn view_lot_outlines<B>(
    context: &crate::nodes::sim::core::RoadPreviewContext,
    take: &crate::nodes::sim::core::RoadLandTake<B>,
) -> PackedVector3Array {
    context.view.with_graded_surface(|surface, scratch| {
        lot_outlines(surface, scratch, take, |lot, at| context.land.lot_corners(lot, at))
    })
}

/// [`lot_outlines`] on the live core, before the take: its graded ground and its parcels.
pub(crate) fn core_lot_outlines(
    core: &SimCore,
    take: &crate::nodes::sim::core::RoadLandTake,
) -> PackedVector3Array {
    core.with_graded_surface(|surface, scratch| {
        lot_outlines(surface, scratch, take, |lot, _| {
            Some(core.zoning.parcel_by_raw_id(lot)?.corners())
        })
    })
}

impl SimulationNode {
    // `replay_road_attempt`'s check and commit of one record.
    fn replay_attempt(&mut self, attempt: &VarDictionary) -> Result<i64, &'static str> {
        // JSON numbers arrive as floats; a missing or mistyped field takes its default.
        let number = |key: &str, default: f64| {
            attempt
                .get(key)
                .and_then(|value| value.try_to::<f64>().ok())
                .unwrap_or(default)
        };
        let flag = |key: &str, default: bool| {
            attempt
                .get(key)
                .and_then(|value| value.try_to::<bool>().ok())
                .unwrap_or(default)
        };
        let points: PackedVector3Array = attempt
            .get("points")
            .and_then(|points| points.try_to::<VarArray>().ok())
            .map(|points| {
                points
                    .iter_shared()
                    .map(|point| {
                        let point = point.try_to::<VarArray>().unwrap_or_default();
                        let coordinate = |index: usize| {
                            point
                                .get(index)
                                .and_then(|value| value.try_to::<f64>().ok())
                                .unwrap_or(0.0) as f32
                        };
                        Vector3::new(coordinate(0), coordinate(1), coordinate(2))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let lanes = (number("fwd_lanes", 1.0) as i32, number("bkw_lanes", 1.0) as i32);
        let snap = flag("snap", true);
        let tunnel = if flag("tunnel", false) { 3.0 } else { 0.0 };
        let profile = number("profile", tunnel) as i32;
        let stroke = (
            RoadMedian::from_tool(
                number("median_style", 0.0) as i32,
                number("median_width", 0.0) as f32,
            )?,
            RoadParking::from_code(number("parking", 0.0) as i32)
                .ok_or("parking_code_out_of_range")?,
            RoadAttributes::from_tool(
                number("speed_kmh", 50.0) as i64,
                number("sidewalks", 3.0) as i64,
                flag("zoning", true),
                flag("street_lights", true),
            )?,
        );
        if points.len() < 2 {
            return Err("too_short");
        }
        let profile = RoadProfile::from_code(profile).ok_or("road_profile_unknown")?;
        if profile != RoadProfile::FollowGround {
            let (median, parking, attributes) = stroke;
            let section = DraftSection {
                fwd_lanes: lanes.0,
                bkw_lanes: lanes.1,
                snap_to_existing_roads: snap,
                median,
                parking,
                attributes,
            };
            let stroke = DraftStroke {
                points: points.to_vec(),
                profile,
            };
            return Ok(self.send_road_batch(draft_batch(vec![stroke], section)));
        }
        let points_vec = points.to_vec();
        let (prepared, validation, surface_generation) =
            self.plan_commit_check(&points_vec, lanes, stroke, snap);
        if !validation.is_valid {
            return Err(validation.invalid_reason);
        }
        let (median, parking, attributes) = stroke;
        let clearance = self.corridor_clearance(
            &prepared.points,
            lanes,
            RoadPreviewRequest {
                surface_generation,
                points: points_vec,
                fwd_lanes: lanes.0,
                bkw_lanes: lanes.1,
                snap_to_existing_roads: snap,
                profile,
                median,
                parking,
                attributes,
            },
        );
        if let Some(reason) = Self::corridor_refusal(&clearance) {
            return Err(reason);
        }
        Ok(self.add_road_with_section(points, lanes, snap, stroke))
    }
}
