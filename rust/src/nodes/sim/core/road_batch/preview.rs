// SPDX-License-Identifier: GPL-2.0-only

//! Batch preview (`ROAD-112` round G1, plan 6.2, `docs/roads.md` "Road tools"): a draft that is
//! several strokes (automatic bridge and tunnel segments, later grids and parallel copies) is
//! previewed in two tiers inside the frame.
//!
//! **Tier 1, every pose**: each stroke's hover check on the published view
//! ([`RoadToolQuerySnapshot::validate`]: preparation, profile, ends, water), independently and
//! in parallel, giving its prepared points (the ghost), class (its exact cost) and validity.
//! `O(K * stroke points)`, no topology and no pieces.
//!
//! **Tier 2, while the pointer rests**: the commit's own plan and pieces, one stroke a frame, in
//! the batch's order on the preview's replica: each stroke planned on the graph and roadbed the
//! earlier ones left, applied, its closure compiled into an `O(1)` copy of the view's roadbed and
//! its land judged as the preview judges it. The replica is put back at the end of every frame
//! (the context's invariant: between calls it equals the view's graph), and the next frame
//! replays the done strokes' records forward first, `O(their local records)`. A new pose
//! restarts the batch; nothing needs undoing then. The frame that accepts the last stroke also
//! stages the pose's graded tiles, as a single stroke's preview does. The exact overlay of the
//! changed chunks follows over the next frames (round G1b), on the replica with every stroke
//! replayed: the batch's owners in emission order cut into one group per stroke at equal shares
//! of their triangles, one group emitted a frame and appended to the chunks
//! ([`PreviewOverlayParts`]), so no frame's payload grows with the batch. The overlay is handed
//! over whole on the last of them, so the batch never shows half.
//!
//! **Commit**: a finished, accepted tier 2 whose fingerprint chain is current hands its plans and
//! pieces to the batch commit ([`BatchPreviewProducts`]), which then skips planning and
//! compiling in its dry run and keeps only the live checks (decision G1-D1 in the round record).
//!
//! **Caps** ([`BATCH_MAX_STROKES`], [`BATCH_MAX_LENGTH_M`]): a larger batch is refused as
//! [`BATCH_TOO_LARGE_REASON`] before any tier runs, never trimmed.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use godot::prelude::Vector3;
use rayon::prelude::*;

use super::super::road_commit::RoadCommitRequest;
use super::super::road_edit_plan::{RoadEditPlan, RoadLandTake};
use super::super::road_preview::{
    RoadPreviewFingerprint, RoadPreviewRequest, RoadPreviewTimes, RoadToolQuerySnapshot,
};
use super::super::road_tool_context::RoadPreviewContext;
use super::super::road_tool_land::ViewLand;
use crate::nodes::sim::road_tool::validate_road_candidate_against_water;
use crate::simulation::network::graph::RegionGraphUndoDelta;
use crate::simulation::network::render::road::{PreviewChunk, PreviewOverlayParts};
use crate::simulation::network::road_edit::{PreparedRoadInput, RoadPreviewValidation};
use crate::simulation::network::roadbed::{PieceId, RoadbedDelta, RoadbedSystem, SurfaceChunkKey};

/// Most strokes a batch may hold (plan 6.2, decision U6): the hover check of this many strokes
/// fits the 2.0 ms tier-1 bar on the 30k view (measured in round G1, `docs/roads.md`).
pub(crate) const BATCH_MAX_STROKES: usize = 40;

/// Most metres of road a batch may hold, summed over its strokes' drawn polylines.
pub(crate) const BATCH_MAX_LENGTH_M: f64 = 8_000.0;

/// The refusal of a batch over the caps.
pub(crate) const BATCH_TOO_LARGE_REASON: &str = "batch_too_large";

/// A polyline's length in metres. `O(points)`.
pub(crate) fn polyline_length_m(points: &[Vector3]) -> f64 {
    points
        .windows(2)
        .map(|pair| f64::from((pair[1] - pair[0]).length()))
        .sum()
}

/// Whether strokes of these polylines exceed the batch caps. `O(points)`.
pub(crate) fn batch_too_large<'a>(strokes: impl ExactSizeIterator<Item = &'a [Vector3]>) -> bool {
    strokes.len() > BATCH_MAX_STROKES
        || strokes.map(polyline_length_m).sum::<f64>() > BATCH_MAX_LENGTH_M
}

impl RoadToolQuerySnapshot {
    /// Tier 1: every stroke's hover check ([`Self::validate`]) on this view, in the batch's
    /// order, run in parallel (the strokes are independent: no stroke sees another's topology).
    /// `None` when the batch exceeds the caps. `O(K * (stroke points + end lookups)) / workers`.
    pub(crate) fn validate_batch(
        &self,
        strokes: &[RoadPreviewRequest],
    ) -> Option<Vec<(PreparedRoadInput, RoadPreviewValidation)>> {
        if batch_too_large(strokes.iter().map(|stroke| stroke.points.as_slice())) {
            return None;
        }
        Some(crate::thread_pools::install(|| {
            strokes.par_iter().map(|stroke| self.validate(stroke)).collect()
        }))
    }
}

/// One stroke of a batch as tier 2 planned and compiled it on the replica.
pub(crate) struct BatchPreviewStroke {
    pub(crate) plan: RoadEditPlan,
    /// What the commit installs for this stroke.
    pub(crate) delta: RoadbedDelta,
    /// The inputs the plan read: the view's surface generation, the batch roadbed's generation
    /// before this stroke, the terrain.
    pub(crate) fingerprint: RoadPreviewFingerprint,
    /// The land this stroke's pieces take, by the preview's rules.
    pub(crate) take: RoadLandTake<u64>,
    /// Render chunks this stroke's pieces changed.
    pub(crate) changed_chunks: Vec<SurfaceChunkKey>,
    // The replica's records before the stroke (restored newest first at a frame's end) and after
    // it (replayed oldest first at the next frame's start).
    backward: RegionGraphUndoDelta,
    forward: RegionGraphUndoDelta,
}

/// How tier 2 ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BatchVerdict {
    /// Every stroke planned, compiled and took only land a stroke may take.
    Accepted,
    /// The stroke at `stroke` (or the whole batch, `None`, over the caps) is refused: `reason`
    /// is the key the commit's dry run would report.
    Refused {
        /// The refused stroke's index.
        stroke: Option<usize>,
        /// Stable reason key.
        reason: String,
    },
}

/// The exact preview of an accepted batch, built by the frame that finishes it.
pub(crate) struct BatchPreviewPieces {
    /// Every changed render chunk's overlay, as a single stroke's `RoadPreviewPieces::chunks`.
    pub(crate) chunks: Option<BTreeMap<SurfaceChunkKey, PreviewChunk>>,
}

/// Tier 2's state on the road tool's context: the batch being previewed and how far it got.
pub(crate) struct BatchPreview {
    requests: Vec<RoadPreviewRequest>,
    base: RoadPreviewFingerprint,
    // The view's roadbed with the done strokes' pieces installed.
    roadbed: RoadbedSystem,
    /// The done strokes, in the batch's order.
    pub(crate) strokes: Vec<BatchPreviewStroke>,
    /// `Some` once tier 2 ended: `Accepted` once every stroke planned, before its overlay.
    pub(crate) verdict: Option<BatchVerdict>,
    // The accepted batch's overlay being built, one owner group a frame.
    payload: Option<PreviewOverlayParts>,
    /// The exact preview, once an accepted batch's overlay is built.
    pub(crate) pieces: Option<BatchPreviewPieces>,
    /// Each tier-2 frame's stages, in frame order.
    pub(crate) frames: Vec<RoadPreviewTimes>,
    /// The accepted pose's graded tiles still to stage after the accepting frame's share (the
    /// display's catch-up frames stage them, `advance_road_preview_terrain`, as a stroke's).
    pub(crate) graded_pending: usize,
}

impl RoadPreviewContext {
    /// Tier 2's batch, while one stands on this view.
    pub(crate) fn batch(&self) -> Option<&BatchPreview> {
        self.batch.as_ref()
    }
}

impl BatchPreview {
    /// The view's roadbed with the done strokes' pieces installed: every stroke's once tier 2
    /// accepted the batch.
    #[cfg(test)]
    pub(crate) fn roadbed(&self) -> &RoadbedSystem {
        &self.roadbed
    }

    // Whether a frame has tier-2 work: a stroke to plan, or an accepted batch's overlay.
    fn pending(&self) -> bool {
        self.verdict.is_none() || self.payload.is_some()
    }

    /// Whether tier 2 ended accepted for exactly `requests` on the fingerprint `base`.
    fn is_for(&self, requests: &[RoadPreviewRequest], base: RoadPreviewFingerprint) -> bool {
        self.base == base && self.requests == requests
    }
}

/// An accepted tier 2's plans and pieces, handed to the batch commit in place of its dry run's
/// planning and compiling (`ROAD-112` round G1; round F's note).
pub(crate) struct BatchPreviewProducts {
    /// Each stroke's plan, pieces and fingerprint, in the batch's order.
    pub(crate) strokes: Vec<(RoadEditPlan, RoadbedDelta, RoadPreviewFingerprint)>,
}

impl BatchPreviewProducts {
    /// Whether these are the products of exactly `strokes`, the first planned on the `live`
    /// core's inputs. `O(strokes' points)`.
    pub(crate) fn is_for(&self, strokes: &[RoadCommitRequest], live: RoadPreviewFingerprint) -> bool {
        self.strokes.len() == strokes.len()
            && self.strokes.first().is_some_and(|(_, _, first)| *first == live)
            && self.strokes.iter().zip(strokes).all(|((plan, _, _), stroke)| {
                let request = plan.request();
                request.points == stroke.points
                    && request.fwd_lanes == stroke.fwd_lanes
                    && request.bkw_lanes == stroke.bkw_lanes
                    && request.snap_to_existing_roads == stroke.snap_to_existing_roads
                    && request.profile == stroke.profile
                    && request.median == stroke.median
                    && request.parking == stroke.parking
                    && request.attributes == stroke.attributes
            })
    }
}

impl RoadPreviewContext {
    /// Tier 2, one frame while the pointer rests: continues the batch of exactly `requests` on
    /// this view by one stroke, or restarts it when the strokes or the view changed (a new pose).
    /// The replica equals the view's graph again on return. Returns the batch's state. Cost per
    /// frame: one stroke's plan, compile and land judgement, plus the done strokes' local
    /// records replayed and restored; the accepting frame adds the graded staging, and each
    /// frame after it one stroke's chunks of the overlay. The frame's stages are returned when
    /// this call advanced tier 2 (`None` once it ended and its overlay is built).
    pub(crate) fn preview_batch_step(
        &mut self,
        requests: Vec<RoadPreviewRequest>,
    ) -> (&BatchPreview, Option<RoadPreviewTimes>) {
        let base = RoadPreviewFingerprint::of(
            self.view.surface_generation,
            &self.view.roadbed,
            &self.view.terrain,
        );
        if !self
            .batch
            .as_ref()
            .is_some_and(|batch| batch.is_for(&requests, base))
        {
            self.restart_batch(requests, base);
        }
        let mut batch = self.batch.take().expect("a batch was just ensured");
        let frame = batch.pending().then(|| self.step_batch(&mut batch));
        (self.batch.insert(batch), frame)
    }

    /// Drops tier 2's state (a pose that is not a batch, a cancel). `O(1)` beyond the frees: the
    /// replica already equals the view's graph.
    pub(crate) fn clear_batch(&mut self) {
        self.batch = None;
    }

    /// Takes an accepted tier 2's plans and pieces for the commit of exactly `requests` on this
    /// view; `None` (tier 2 unfinished, refused or for other strokes) leaves the commit's dry run
    /// to plan. The context's batch goes with them.
    pub(crate) fn take_batch_products(
        &mut self,
        requests: &[RoadPreviewRequest],
    ) -> Option<BatchPreviewProducts> {
        let base = RoadPreviewFingerprint::of(
            self.view.surface_generation,
            &self.view.roadbed,
            &self.view.terrain,
        );
        let accepted = self.batch.as_ref().is_some_and(|batch| {
            batch.is_for(requests, base) && batch.verdict == Some(BatchVerdict::Accepted)
        });
        if !accepted {
            return None;
        }
        let batch = self.batch.take()?;
        Some(BatchPreviewProducts {
            strokes: batch
                .strokes
                .into_iter()
                .map(|stroke| (stroke.plan, stroke.delta, stroke.fingerprint))
                .collect(),
        })
    }

    fn restart_batch(&mut self, requests: Vec<RoadPreviewRequest>, base: RoadPreviewFingerprint) {
        // The last pose's graded tiles are not this batch's (T60 (2)).
        self.graded.supersede();
        let too_large = batch_too_large(requests.iter().map(|stroke| stroke.points.as_slice()));
        self.batch = Some(BatchPreview {
            roadbed: self.view.roadbed.snapshot(),
            strokes: Vec::with_capacity(requests.len().min(BATCH_MAX_STROKES)),
            verdict: too_large.then(|| BatchVerdict::Refused {
                stroke: None,
                reason: BATCH_TOO_LARGE_REASON.to_owned(),
            }),
            requests,
            base,
            payload: None,
            pieces: None,
            frames: Vec::new(),
            graded_pending: 0,
        });
    }

    // One stroke of tier 2, or one stroke's chunks of an accepted batch's overlay (module
    // documentation), with the replica replayed to the done strokes' state first and restored
    // to the view's graph after.
    fn step_batch(&mut self, batch: &mut BatchPreview) -> RoadPreviewTimes {
        let plan_start = Instant::now();
        for stroke in &batch.strokes {
            self.replica.restore_undo_delta_local(&stroke.forward);
        }
        let mut times = RoadPreviewTimes::default();
        if let Some(parts) = batch.payload.as_mut() {
            times.roadbed_ms = plan_start.elapsed().as_secs_f64() * 1000.0;
            let payload_start = Instant::now();
            let sites = super::super::graded_stage::PreviewSites::new(
                &self.view.terrain,
                &self.view.terrain_inputs,
            );
            let built = parts.build_next(
                &self.replica,
                &self.view.terrain,
                &sites,
                &batch.roadbed,
                &self.view.road_mesh_chunks,
            );
            if built.is_none() || parts.is_done() {
                let parts = batch.payload.take().expect("the parts being built");
                let chunks = built.map(|()| parts.finish());
                batch.pieces = Some(BatchPreviewPieces { chunks });
            }
            times.payload_ms = payload_start.elapsed().as_secs_f64() * 1000.0;
        } else {
            let index = batch.strokes.len();
            let outcome = self.batch_stroke(batch, index, plan_start, &mut times);
            match outcome {
                Err(reason) => {
                    batch.verdict = Some(BatchVerdict::Refused {
                        stroke: Some(index),
                        reason,
                    });
                }
                Ok(stroke) => {
                    batch.strokes.push(stroke);
                    if batch.strokes.len() == batch.requests.len() {
                        self.accept_batch(batch, &mut times);
                        batch.verdict = Some(BatchVerdict::Accepted);
                    }
                }
            }
        }
        let restore_start = Instant::now();
        for stroke in batch.strokes.iter().rev() {
            self.replica.restore_undo_delta_local(&stroke.backward);
        }
        times.roadbed_ms += restore_start.elapsed().as_secs_f64() * 1000.0;
        batch.frames.push(times);
        times
    }

    // Plans, applies, compiles and judges stroke `index` on the replica as the earlier strokes
    // left it, installing its pieces into the batch's roadbed. On a refusal the stroke's own
    // records are already put back. The refusal's key is the commit dry run's.
    fn batch_stroke(
        &mut self,
        batch: &mut BatchPreview,
        index: usize,
        plan_start: Instant,
        times: &mut RoadPreviewTimes,
    ) -> Result<BatchPreviewStroke, String> {
        let fingerprint = RoadPreviewFingerprint::of(
            self.view.surface_generation,
            &batch.roadbed,
            &self.view.terrain,
        );
        let plan = RoadEditPlan::compile_from(
            &self.view.terrain,
            &self.replica,
            &batch.roadbed,
            batch.requests[index].clone(),
        );
        let validation = {
            let request = plan.request();
            validate_road_candidate_against_water(
                plan.prepared(),
                request.fwd_lanes.clamp(0, i32::from(u8::MAX)) as u8,
                request.bkw_lanes.clamp(0, i32::from(u8::MAX)) as u8,
                &self.view.water,
                plan.validation().clone(),
            )
        };
        times.plan_ms = plan_start.elapsed().as_secs_f64() * 1000.0;
        if !validation.is_valid {
            return Err(validation.invalid_reason.to_owned());
        }
        let roadbed_start = Instant::now();
        let Some(topology) = plan.topology_for(&self.replica) else {
            return Err("road_plan_provisional".to_owned());
        };
        let (edges, nodes) = topology.source_topology_ids();
        let backward = self.replica.capture_undo_for_local_topology(edges, nodes);
        let Some(applied) = topology.apply_to_graph(&mut self.replica) else {
            self.replica.restore_undo_delta_local(&backward);
            return Err("road_plan_stale".to_owned());
        };
        applied.mark_pieces(&mut batch.roadbed);
        let delta = batch
            .roadbed
            .compile_delta_with(&self.replica, &mut self.roadbed_scratch);
        let (changed_chunks, restore) = batch.roadbed.install(&delta);
        let before: Vec<_> = restore.replaced_pieces().collect();
        let installed: Vec<_> = batch.roadbed.installed_pieces(&restore).collect();
        let take = ViewLand {
            land: &self.land,
            cells: &self.view.cells,
        }
        .take(&installed, &before);
        let refusal = match (take.plotted, batch.roadbed.failed_nodes().first()) {
            (Some(plotted), _) => Some(plotted.reason().to_owned()),
            (None, Some(_)) => Some("render_geometry_invalid".to_owned()),
            (None, None) => None,
        };
        let (node_count, edge_count) = (self.replica.node_count(), self.replica.edge_count());
        let (edges, nodes) = backward.affected_topology_ids(node_count, edge_count);
        let forward = self.replica.capture_undo_for_local_topology(edges, nodes);
        times.roadbed_ms = roadbed_start.elapsed().as_secs_f64() * 1000.0;
        if let Some(reason) = refusal {
            self.replica.restore_undo_delta_local(&backward);
            return Err(reason);
        }
        Ok(BatchPreviewStroke {
            plan,
            delta,
            fingerprint,
            take,
            changed_chunks,
            backward,
            forward,
        })
    }

    /// Test builds: the finished batch's overlay built in one call over every chunk its strokes
    /// changed, as round G1 built it on the accepting frame; the replica equals the view's graph
    /// again on return. `None` without a finished batch.
    #[cfg(test)]
    pub(crate) fn batch_overlay_in_one_call(
        &mut self,
    ) -> Option<Option<BTreeMap<SurfaceChunkKey, PreviewChunk>>> {
        let batch = self.batch.take()?;
        let finished = batch.strokes.len() == batch.requests.len();
        let closure: Vec<_> = batch_owners(&batch.strokes)
            .into_iter()
            .map(|id| (id, batch.roadbed.piece_mesh(id).is_some()))
            .collect();
        let target: BTreeSet<_> = batch
            .strokes
            .iter()
            .flat_map(|stroke| stroke.changed_chunks.iter().copied())
            .collect();
        for stroke in &batch.strokes {
            self.replica.restore_undo_delta_local(&stroke.forward);
        }
        let sites = super::super::graded_stage::PreviewSites::new(
            &self.view.terrain,
            &self.view.terrain_inputs,
        );
        let chunks = crate::simulation::network::render::road::RoadRenderer
            .generate_preview_overlay(
                &self.replica,
                &self.view.terrain,
                &sites,
                &batch.roadbed,
                &target,
                &closure,
                self.view.roadbed.generation(),
                &self.view.road_mesh_chunks,
            );
        for stroke in batch.strokes.iter().rev() {
            self.replica.restore_undo_delta_local(&stroke.backward);
        }
        self.batch = Some(batch);
        finished.then_some(chunks)
    }

    // The accepting frame, on the replica with every stroke applied: the pose's graded tiles,
    // as a single stroke's preview stages them from its one delta (`compile_plan_pieces`), over
    // the union of the strokes' deltas, and the overlay's closure for the frames after.
    fn accept_batch(&mut self, batch: &mut BatchPreview, times: &mut RoadPreviewTimes) {
        let owners = batch_owners(&batch.strokes);
        let terrain_changes = batch.roadbed.take_terrain_changes();
        let graded_start = Instant::now();
        let end = batch
            .requests
            .last()
            .and_then(|request| request.points.last())
            .map_or([0.0; 2], |end| [f64::from(end.x), f64::from(end.z)]);
        // The shown tiles are staged as a stroke's pose stages them; the commit stages its own.
        batch.graded_pending = self.graded.stage_pose(
            &self.view.terrain,
            &self.view.roadbed,
            &batch.roadbed,
            &self.view.terrain_inputs,
            &terrain_changes,
            owners.iter().copied(),
            end,
        )
        .pending;
        times.graded_ms = graded_start.elapsed().as_secs_f64() * 1000.0;
        // The overlay composes over the published chunks only while they are the view's.
        if self.view.road_mesh_generation != Some(self.view.roadbed.generation()) {
            batch.pieces = Some(BatchPreviewPieces { chunks: None });
            return;
        }
        let target = batch
            .strokes
            .iter()
            .flat_map(|stroke| stroke.changed_chunks.iter().copied())
            .collect();
        // With whether the batch's roadbed now holds a piece for each (a later stroke may
        // rebuild an earlier one's piece).
        let closure = owners
            .iter()
            .map(|&id| (id, batch.roadbed.piece_mesh(id).is_some()))
            .collect();
        batch.payload = Some(PreviewOverlayParts::new(
            &batch.roadbed,
            target,
            closure,
            batch.strokes.len(),
        ));
    }
}

// Every owner a stroke of the batch rebuilt or failed, ascending (the emission order).
fn batch_owners(strokes: &[BatchPreviewStroke]) -> BTreeSet<PieceId> {
    strokes
        .iter()
        .flat_map(|stroke| {
            let delta = &stroke.delta;
            delta
                .pieces()
                .map(|(id, _)| id)
                .chain(delta.failed_nodes().map(|(node, _)| PieceId::Node(node)))
        })
        .collect()
}
