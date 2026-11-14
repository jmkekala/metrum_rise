// SPDX-License-Identifier: GPL-2.0-only

//! Batch preview Godot API (`ROAD-112` round G1, plan 6.2): a draft that segments into several
//! strokes (`segment_draft`, round H) previews as a batch. Every pose runs tier 1 (each stroke's
//! hover check and a ribbon ghost; exact cost and length); a resting pointer also runs one tier-2
//! stroke a frame on the preview's replica, then the exact overlay in as many frames (a
//! stroke's share of its triangles each), and the frame that completes it returns the overlay
//! as `preview_road` does. The commit of the same draft (`add_road_draft`) takes a
//! finished tier 2's plans and pieces in place of its dry run's planning. Takes the context lock
//! only, never the core lock.

use super::super::super::*;
use crate::nodes::sim::core::{
    BATCH_TOO_LARGE_REASON, BatchPreview, BatchVerdict, polyline_length_m,
};
use crate::nodes::sim::road_tool::batch::{draft_requests, ghost_half_width_m, push_ghost_ribbon};
use crate::nodes::sim::road_tool::height::segment_draft;
use crate::simulation::network::road_types::cost as road_cost;

#[godot_api(secondary)]
impl SimulationNode {
    /// Previews a draft along `path` whose grade line runs from `start_m` to `end_m` (world
    /// heights at its ends) as the batch its segmentation makes (`ROAD-112` round G1): the
    /// strokes `add_road_draft` would queue, on the road tool's published view. Tier 1 runs on
    /// every call; with `resting` (the pointer did not move since the last call) tier 2 advances
    /// by one stroke. Returns `reason` (empty, the segmentation's refusal such as `cut_too_deep`,
    /// or `batch_too_large` over the caps of 40 strokes and 8 km), and with strokes:
    /// `stroke_count`, `profiles` (`RoadProfile::code` each), `valid` (tier 1's verdict each),
    /// `reasons` (each stroke's hover refusal, empty when valid), `costs` and `lengths_m` (each
    /// stroke's exact build cost and prepared length), `build_cost` and `length_m` (their sums),
    /// `ghost` (a triangle list: every stroke's ribbon at its road's width) with `ghost_starts`
    /// (each stroke's first vertex), `tier1_ms`; and `tier2_done` (strokes tier 2 finished),
    /// `tier2_state` (`running`, `accepted` or `refused`), `tier2_reason` and `tier2_stroke` (a
    /// refusal's key and stroke, -1 for the whole batch), `tier2_ms` (this frame's tier-2 work)
    /// with its stages `tier2_plan_ms`, `tier2_roadbed_ms`, `tier2_payload_ms`,
    /// `tier2_graded_ms`. Once every stroke is planned, the resting frames after build the exact
    /// overlay, a stroke's share of its triangles a frame (round G1b), and `tier2_state` stays
    /// `running` until it is whole. The frame that turns it `accepted` adds what `preview_road`
    /// returns for a stroke's pieces: `has_road_chunks`, `road_chunks`, `road_mesh_generation`,
    /// `chunk_span_m`, `chunk_origin_x_m`, `chunk_origin_z_m`, `changed_chunks`, the graded
    /// tiles' changes, `demolish_count` and `demolish_lots`.
    #[func]
    #[allow(clippy::too_many_arguments)]
    pub fn preview_road_draft(
        &self,
        path: PackedVector3Array,
        start_m: f32,
        end_m: f32,
        fwd_lanes: i32,
        bkw_lanes: i32,
        snap_to_existing_roads: bool,
        resting: bool,
    ) -> VarDictionary {
        let section = self.road_draft_section(fwd_lanes, bkw_lanes, snap_to_existing_roads);
        let mut dict = VarDictionary::new();
        let requests = {
            let view = self.road_tool.view.read().unwrap();
            segment_draft(
                &path.to_vec(),
                [start_m, end_m],
                section,
                &view.terrain,
                &view.region_graph,
                &view.roadbed,
            )
            .map(|strokes| draft_requests(&strokes, section, view.surface_generation))
        };
        match requests {
            Ok(requests) => self.preview_road_batch_requests(requests, resting, &mut dict),
            Err(reason) => dict.set("reason", reason),
        }
        dict
    }

    // Both tiers of one batch pose into `dict` (`preview_road_draft`'s keys).
    pub(super) fn preview_road_batch_requests(
        &self,
        requests: Vec<RoadPreviewRequest>,
        resting: bool,
        dict: &mut VarDictionary,
    ) {
        let tier1_start = std::time::Instant::now();
        let hover = self.road_tool.view.read().unwrap().validate_batch(&requests);
        let Some(hover) = hover else {
            dict.set("reason", BATCH_TOO_LARGE_REASON);
            self.road_tool
                .context
                .lock()
                .expect("road tool context lock poisoned")
                .clear_batch();
            return;
        };
        let mut ghost = Vec::new();
        let mut ghost_starts = PackedInt32Array::new();
        let (mut costs, mut lengths) = (PackedFloat64Array::new(), PackedFloat64Array::new());
        let mut valid = PackedInt32Array::new();
        let mut reasons = PackedStringArray::new();
        let mut profiles = PackedInt32Array::new();
        for (request, (prepared, validation)) in requests.iter().zip(&hover) {
            ghost_starts.push(ghost.len() as i32);
            push_ghost_ribbon(prepared, ghost_half_width_m(request), &mut ghost);
            let weight =
                Self::stroke_cost_weight((request.fwd_lanes, request.bkw_lanes), prepared);
            let length_m = polyline_length_m(&prepared.points);
            costs.push(length_m * road_cost::build_per_m(weight));
            lengths.push(length_m);
            valid.push(i32::from(validation.is_valid));
            reasons.push(if validation.is_valid {
                ""
            } else {
                validation.invalid_reason
            });
            profiles.push(request.profile.code());
        }
        dict.set("reason", "");
        dict.set("stroke_count", requests.len() as i64);
        dict.set("build_cost", costs.as_slice().iter().sum::<f64>());
        dict.set("length_m", lengths.as_slice().iter().sum::<f64>());
        dict.set("costs", costs);
        dict.set("lengths_m", lengths);
        dict.set("valid", valid);
        dict.set("reasons", reasons);
        dict.set("profiles", profiles);
        dict.set("ghost", PackedVector3Array::from(ghost.as_slice()));
        dict.set("ghost_starts", ghost_starts);
        dict.set("tier1_ms", tier1_start.elapsed().as_secs_f64() * 1000.0);
        let mut context = self
            .road_tool
            .context
            .lock()
            .expect("road tool context lock poisoned");
        if !resting {
            // A new pose restarts tier 2 when the pointer next rests.
            context.clear_batch();
            dict.set("tier2_state", "running");
            dict.set("tier2_done", 0_i64);
            return;
        }
        context.land = self.road_tool.land();
        let (batch, frame) = context.preview_batch_step(requests);
        Self::set_tier2(dict, batch, frame.unwrap_or_default());
        // The exact preview goes once, on the frame that completed the accepted batch's overlay.
        if frame.is_some() && batch.pieces.is_some() {
            Self::set_batch_pieces(&mut context, dict);
        }
    }

    // Tier 2's state and this frame's stages into `dict`.
    fn set_tier2(
        dict: &mut VarDictionary,
        batch: &BatchPreview,
        frame: crate::nodes::sim::core::RoadPreviewTimes,
    ) {
        let (state, reason, stroke) = match &batch.verdict {
            // An accepted batch is shown once its overlay is whole.
            None => ("running", "", -1),
            Some(BatchVerdict::Accepted) if batch.pieces.is_none() => ("running", "", -1),
            Some(BatchVerdict::Accepted) => ("accepted", "", -1),
            Some(BatchVerdict::Refused { stroke, reason }) => (
                "refused",
                reason.as_str(),
                stroke.map_or(-1, |stroke| stroke as i64),
            ),
        };
        dict.set("tier2_state", state);
        dict.set("tier2_reason", reason);
        dict.set("tier2_stroke", stroke);
        dict.set("tier2_done", batch.strokes.len() as i64);
        dict.set(
            "tier2_ms",
            frame.plan_ms + frame.roadbed_ms + frame.payload_ms + frame.graded_ms,
        );
        dict.set("tier2_plan_ms", frame.plan_ms);
        dict.set("tier2_roadbed_ms", frame.roadbed_ms);
        dict.set("tier2_payload_ms", frame.payload_ms);
        dict.set("tier2_graded_ms", frame.graded_ms);
    }

    // The accepted batch's exact preview into `dict`, keyed as `preview_road` keys a stroke's.
    fn set_batch_pieces(
        context: &mut crate::nodes::sim::core::RoadPreviewContext,
        dict: &mut VarDictionary,
    ) {
        let (mut tiles, mut removed) = (Vec::new(), Vec::new());
        context.graded.take_shown_changes(&mut tiles, &mut removed);
        let cell_m = f64::from(context.view.terrain.cell_size_m());
        Self::set_graded_preview_changes(dict, &tiles, &removed, context.graded.epoch(), cell_m);
        let batch = context.batch().expect("an accepted batch");
        let pieces = batch.pieces.as_ref().expect("an accepted batch's pieces");
        dict.set("has_road_chunks", pieces.chunks.is_some());
        dict.set(
            "road_chunks",
            pieces
                .chunks
                .as_ref()
                .map(Self::preview_road_chunk_updates)
                .unwrap_or_default(),
        );
        let mut changed = PackedInt32Array::new();
        let mut demolish_count = 0;
        let mut demolish_lots = PackedVector3Array::new();
        for stroke in &batch.strokes {
            for &(x, z) in &stroke.changed_chunks {
                changed.push(x);
                changed.push(z);
            }
            demolish_count += stroke.take.buildings.len();
            demolish_lots.extend_array(&super::view_lot_outlines(context, &stroke.take));
        }
        dict.set("changed_chunks", changed);
        dict.set("demolish_count", demolish_count as i64);
        dict.set("demolish_lots", demolish_lots);
        let roadbed = &context.view.roadbed;
        let (origin_x, origin_z) = roadbed.chunk_origin_m();
        dict.set(
            "road_mesh_generation",
            i64::try_from(context.view.road_mesh_surface_generation).unwrap_or(i64::MAX),
        );
        dict.set("chunk_span_m", roadbed.chunk_span_m());
        dict.set("chunk_origin_x_m", origin_x);
        dict.set("chunk_origin_z_m", origin_z);
    }
}
