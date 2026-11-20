// SPDX-License-Identifier: GPL-3.0-or-later

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
use crate::nodes::sim::road_tool::batch::{
    draft_requests, ghost_half_width_m, push_ghost_edges, push_ghost_ribbon,
};
use crate::nodes::sim::road_tool::draw::Ghost;
use crate::nodes::sim::road_tool::draw::chunks::PreviewOverlay;
use crate::nodes::sim::road_tool::draw::lines::LineStyle;
use crate::nodes::sim::road_tool::height::segment_draft;
use crate::nodes::sim::road_tool::hue::{FootprintHue, HueKind, HueState};
use crate::nodes::sim::road_tool::outcome::Tier2State;
use std::borrow::Cow;
use crate::simulation::network::road_edit::DraftGround;
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
    /// (each stroke's first vertex), the ribbons' edges as `road_tool_line.gdshader` quads
    /// (round V2; as `road_tool_snapshot`'s `gizmo_line_*`, looks 5 accepted and 6 refused)
    /// `ghost_line_vertices` and `ghost_line_attributes`,
    /// `tier1_ms`; and `tier2_done` (strokes tier 2 finished),
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
        &mut self,
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
        // The ghost stands as the tool's last tier 1 (`road_tool_draw_batch_ghost` draws it).
        let mut ghost = std::mem::take(&mut self.road_tool_session.ghost);
        match requests {
            Ok(requests) => {
                self.preview_road_batch_requests(&requests, resting, &mut dict, &mut ghost)
            }
            Err(reason) => {
                ghost.clear();
                dict.set("reason", reason);
            }
        }
        self.road_tool_session.ghost = ghost;
        dict
    }

    // Both tiers of one batch pose into `dict` (`preview_road_draft`'s keys).
    pub(super) fn preview_road_batch_requests(
        &self,
        requests: &[RoadPreviewRequest],
        resting: bool,
        dict: &mut VarDictionary,
        ghost: &mut Ghost,
    ) {
        if (self.preview_road_batch_tier1(requests, Some(&mut *dict), None, ghost)).is_none() {
            return;
        }
        if !resting {
            // A new pose restarts tier 2 when the pointer next rests.
            self.road_tool
                .context
                .lock()
                .expect("road tool context lock poisoned")
                .clear_batch();
            dict.set("tier2_state", "running");
            dict.set("tier2_done", 0_i64);
            return;
        }
        let step = self.preview_road_batch_tier2(requests, dict);
        if step.overlay.is_some() {
            // `preview_road_draft`'s callers (tests) read the accepted batch's payload.
            let context = self
                .road_tool
                .context
                .lock()
                .expect("road tool context lock poisoned");
            let chunks = (context.batch())
                .and_then(|batch| batch.pieces.as_ref())
                .and_then(|pieces| pieces.chunks.as_ref())
                .map(Self::preview_road_chunk_updates)
                .unwrap_or_default();
            dict.set("road_chunks", chunks);
        }
    }

    // Tier 1 of a batch pose: the strokes' hover verdicts, costs, lengths and the ghost into
    // `ghost` (round R2-3: typed, drawn by the tool's drawing), and with `dict` its keys
    // (`reason`, the verdicts, costs, lengths and the ghost as `preview_road_draft` returns
    // them); with `hue`, the strokes' footprint hue (round HU: checking while every stroke
    // passes, else refused). Returns its verdict typed (round R2-1); `None` (tier 2 dropped, the
    // ghost empty) over the caps, the hue untouched.
    pub(super) fn preview_road_batch_tier1(
        &self,
        requests: &[RoadPreviewRequest],
        dict: Option<&mut VarDictionary>,
        hue: Option<&mut FootprintHue>,
        ghost: &mut Ghost,
    ) -> Option<Tier1> {
        let tier1_start = std::time::Instant::now();
        ghost.clear();
        let view = self.road_tool.view.read().unwrap();
        let hover = view.validate_batch(requests);
        let Some(hover) = hover else {
            drop(view);
            if let Some(dict) = dict {
                dict.set("reason", BATCH_TOO_LARGE_REASON);
            }
            self.road_tool
                .context
                .lock()
                .expect("road tool context lock poisoned")
                .clear_batch();
            return None;
        };
        let (mut costs, mut lengths) = (PackedFloat64Array::new(), PackedFloat64Array::new());
        let mut reasons = PackedStringArray::new();
        let mut profiles = PackedInt32Array::new();
        // The first stroke with a refusal key, and that key.
        let mut refusal = None;
        let (mut build_cost, mut total_length_m) = (0.0, 0.0);
        for (stroke, (request, (prepared, validation))) in requests.iter().zip(&hover).enumerate() {
            ghost.starts.push(ghost.ribbon.len() as u32);
            let half_width_m = ghost_half_width_m(request);
            push_ghost_ribbon(prepared, half_width_m, &mut ghost.ribbon);
            push_ghost_edges(&prepared.points, half_width_m, &mut ghost.edges, &mut ghost.edge_starts);
            let weight =
                Self::stroke_cost_weight((request.fwd_lanes, request.bkw_lanes), prepared);
            let length_m = polyline_length_m(&prepared.points);
            let cost = length_m * road_cost::build_per_m(weight);
            build_cost += cost;
            total_length_m += length_m;
            ghost.valid.push(validation.is_valid);
            let reason = if validation.is_valid {
                ""
            } else {
                validation.invalid_reason
            };
            if refusal.is_none() && !reason.is_empty() {
                refusal = Some((stroke, reason));
            }
            if dict.is_some() {
                costs.push(cost);
                lengths.push(length_m);
                reasons.push(reason);
                profiles.push(request.profile.code());
            }
        }
        if let (Some(hue), Some(first)) = (hue, requests.first()) {
            let ground = DraftGround::new(&view.terrain, &view.roadbed);
            hue.begin();
            for (request, (prepared, _)) in requests.iter().zip(&hover) {
                hue.push_stroke(
                    &prepared.points,
                    HueKind::of(request.profile, prepared.class),
                );
            }
            let state = if ghost.valid.iter().all(|&valid| valid) {
                HueState::Checking
            } else {
                HueState::Refused
            };
            // Every stroke of a draft shares its section, so its width.
            hue.finish(
                ghost_half_width_m(first),
                |x, z| ground.height_at(x, z),
                // Tier 1 draws no pieces: the strokes' strips are the fill.
                |_, _| f64::NEG_INFINITY,
                state,
            );
        }
        drop(view);
        let tier1 = Tier1 {
            refusal,
            build_cost,
            length_m: total_length_m,
        };
        if let Some(dict) = dict {
            // Two edge strips a stroke, in the ghost's look by tier 1's verdict.
            let edge_quads = super::session::strip_quads(&ghost.edges, &ghost.edge_starts, |strip| {
                if ghost.valid[strip / 2] {
                    LineStyle::GhostEdge
                } else {
                    LineStyle::GhostRefusedEdge
                }
            });
            dict.set("ghost_line_vertices", edge_quads.vertices);
            dict.set("ghost_line_attributes", edge_quads.attributes);
            dict.set("reason", "");
            dict.set("stroke_count", requests.len() as i64);
            dict.set("build_cost", tier1.build_cost);
            dict.set("length_m", tier1.length_m);
            dict.set("costs", costs);
            dict.set("lengths_m", lengths);
            let valid = ghost.valid.iter().map(|&valid| i32::from(valid));
            dict.set("valid", valid.collect::<PackedInt32Array>());
            dict.set("reasons", reasons);
            dict.set("profiles", profiles);
            dict.set("ghost", PackedVector3Array::from(ghost.ribbon.as_slice()));
            let starts = ghost.starts.iter().map(|&start| start as i32);
            dict.set("ghost_starts", starts.collect::<PackedInt32Array>());
            dict.set("tier1_ms", tier1_start.elapsed().as_secs_f64() * 1000.0);
        }
        Some(tier1)
    }

    // One tier-2 step of a resting batch pose into `dict` (`tier2_*`, and on the frame that
    // completes an accepted batch's overlay its exact preview); returns the step typed (round
    // R2-1).
    pub(super) fn preview_road_batch_tier2(
        &self,
        requests: &[RoadPreviewRequest],
        dict: &mut VarDictionary,
    ) -> Tier2Step {
        let mut context = self
            .road_tool
            .context
            .lock()
            .expect("road tool context lock poisoned");
        context.land = self.road_tool.land();
        let (batch, frame) = context.preview_batch_step(requests);
        let mut step = Self::set_tier2(dict, batch, frame.unwrap_or_default());
        // The exact preview goes once, on the frame that completed the accepted batch's overlay.
        if frame.is_some() && batch.pieces.is_some() {
            (step.demolish_count, step.overlay) = Self::set_batch_pieces(&mut context, dict);
        }
        step
    }

    // Tier 2's state and this frame's stages into `dict`, and the state typed.
    fn set_tier2(
        dict: &mut VarDictionary,
        batch: &BatchPreview,
        frame: crate::nodes::sim::core::RoadPreviewTimes,
    ) -> Tier2Step {
        let (state, reason, stroke) = match &batch.verdict {
            // An accepted batch is shown once its overlay is whole.
            None => (Tier2State::Running, "", None),
            Some(BatchVerdict::Accepted) if batch.pieces.is_none() => {
                (Tier2State::Running, "", None)
            }
            Some(BatchVerdict::Accepted) => (Tier2State::Accepted, "", None),
            Some(BatchVerdict::Refused { stroke, reason }) => {
                (Tier2State::Refused, reason.as_str(), *stroke)
            }
        };
        dict.set("tier2_state", state.key());
        dict.set("tier2_reason", reason);
        dict.set("tier2_stroke", stroke.map_or(-1, |stroke| stroke as i64));
        dict.set("tier2_done", batch.strokes.len() as i64);
        dict.set(
            "tier2_ms",
            frame.plan_ms + frame.roadbed_ms + frame.payload_ms + frame.graded_ms,
        );
        dict.set("tier2_plan_ms", frame.plan_ms);
        dict.set("tier2_roadbed_ms", frame.roadbed_ms);
        dict.set("tier2_payload_ms", frame.payload_ms);
        dict.set("tier2_graded_ms", frame.graded_ms);
        Tier2Step {
            state,
            // Only a refusal carries a key (one copy, on its frame).
            reason: if reason.is_empty() {
                Cow::Borrowed("")
            } else {
                Cow::Owned(reason.to_owned())
            },
            stroke,
            demolish_count: 0,
            overlay: None,
        }
    }

    // The accepted batch's exact preview into `dict`, keyed as `preview_road` keys a stroke's;
    // returns the buildings it demolishes and its overlay chunks encoded for the draw module.
    fn set_batch_pieces(
        context: &mut crate::nodes::sim::core::RoadPreviewContext,
        dict: &mut VarDictionary,
    ) -> (i64, Option<std::sync::Arc<PreviewOverlay>>) {
        let mut changes = crate::nodes::sim::core::ShownChanges::default();
        context.graded.take_shown_changes(&mut changes);
        Self::set_graded_preview_changes(dict, &changes, context.graded.epoch());
        let batch = context.batch().expect("an accepted batch");
        let pieces = batch.pieces.as_ref().expect("an accepted batch's pieces");
        dict.set("has_road_chunks", pieces.chunks.is_some());
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
        // The pose's tiles the accepting frame left for the display's catch-up frames (round I).
        dict.set("graded_pending", batch.graded_pending as i64);
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
        (demolish_count as i64, pieces.overlay.clone())
    }
}

// Tier 1's verdict of a batch pose within the caps: the first stroke with a refusal key and
// that key, and the strokes' summed build cost and length.
pub(super) struct Tier1 {
    pub(super) refusal: Option<(usize, &'static str)>,
    pub(super) build_cost: f64,
    pub(super) length_m: f64,
}

// One tier-2 step: its state, a refusal's key and stroke (`None` for the whole batch), and the
// buildings an accepted batch demolishes.
pub(super) struct Tier2Step {
    pub(super) state: Tier2State,
    pub(super) reason: Cow<'static, str>,
    pub(super) stroke: Option<usize>,
    pub(super) demolish_count: i64,
    // The accepted batch's overlay chunks, on the frame that completed them.
    pub(super) overlay: Option<std::sync::Arc<PreviewOverlay>>,
}
