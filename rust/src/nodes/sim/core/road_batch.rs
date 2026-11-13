// SPDX-License-Identifier: GPL-2.0-only

//! A batch of road strokes committed as one transaction (`ROAD-112` round F, `docs/roads.md`
//! "Preview, commit, undo and load"): a draft's segments, a grid's lines or parallel copies.
//!
//! **Order is the determinism rule.** The strokes commit in the order the request lists them;
//! each later stroke meets the earlier ones through the same topology pass as a stroke drawn
//! after them, so a batch leaves the graph, roadbed, cells and parcels bitwise as the same strokes
//! committed one by one.
//!
//! **All or nothing, and no land before the verdict.** An undo gives back roads and paint but never
//! demolished buildings, so the batch is first dry-run on the live graph as the preview runs a
//! stroke: each stroke planned on the graph and roadbed the earlier ones left, applied, its
//! closure compiled into an `O(1)` copy of the roadbed and its plotted land judged; then the graph
//! is put back. A refused stroke anywhere ends the command with nothing changed. Otherwise every
//! stroke commits through the single stroke's core, installing its dry run's plan and pieces
//! while the batch's chain holds ([`DryRunStroke::rebased`]), and the command's tail (mesh
//! precompute, publication with its graph copy, routing schedule, lot plan spawn) runs once.
//!
//! **One undo step.** The strokes' records share an undo group; undo restores the group whole.
//!
//! Cost: `O(sum of each stroke's local work)` for the dry run and the cores, plus one tail.

use std::time::Instant;

use super::frame::record_crash_phase_for_core;
use super::road_commit::{RoadCommitOutcome, RoadCommitRequest, StrokePlan, SubmittedStroke};
use super::road_edit_plan::{RoadEditPlan, RoadLandTake};
use super::road_preview::{RoadPreviewFingerprint, RoadPreviewRequest};
use super::road_rejection::RoadRejection;
use super::state::SimCore;
use crate::debug_log;
use crate::nodes::sim::benchmark::road_edit::RoadEditMetrics;
use crate::nodes::sim::road_tool::validate_road_candidate_against_water;
use crate::simulation::network::graph::RegionGraphUndoDelta;
use crate::simulation::network::road_edit::validate_road_candidate_fast;
use crate::simulation::network::roadbed::{RoadbedDelta, RoadbedSystem};

/// An ordered list of strokes committed as one transaction and undone as one step.
pub(crate) struct RoadBatchRequest {
    /// The strokes in commit order (the batch's determinism rule). Their `preview` is not
    /// read: the batch's dry run is each stroke's preview.
    pub(crate) strokes: Vec<RoadCommitRequest>,
}

/// Result of [`SimCore::commit_road_batch`].
pub(crate) struct RoadBatchOutcome {
    /// The command's outcome: committed when every stroke was; otherwise the refused stroke's
    /// reason, with nothing changed.
    pub(crate) outcome: RoadCommitOutcome,
    /// The index of the stroke that refused the batch.
    pub(crate) refused_stroke: Option<usize>,
    /// Wall-clock stages of a batch of more than one stroke, in milliseconds.
    pub(crate) times: RoadBatchTimes,
}

/// Wall-clock stages of one batch command (`ROAD-112` round F), in milliseconds.
#[derive(Clone, Debug, Default)]
pub(crate) struct RoadBatchTimes {
    /// The dry run of every stroke (plan, apply, compile, land judgement, graph restore).
    pub(crate) dry_run_ms: f64,
    /// Each committed stroke's core, in commit order.
    pub(crate) core_ms: Vec<f64>,
    /// The command's one tail.
    pub(crate) tail_ms: f64,
    /// How many strokes installed their dry run's plan and pieces (the batch's chain held).
    pub(crate) reused: usize,
}

/// One stroke of a batch as its dry run planned and compiled it, for the commit to install.
pub(crate) struct DryRunStroke {
    plan: RoadEditPlan,
    delta: RoadbedDelta,
    // The inputs the plan read: the roadbed generation (the dry run's copy) and terrain.
    fingerprint: RoadPreviewFingerprint,
}

impl DryRunStroke {
    /// The plan, stamped with the live surface generation, and the pieces, when the live core
    /// is what the dry run planned this stroke on: the batch's chain. The dry run started from
    /// the live roadbed and each earlier stroke installed its own dry run's pieces, so an equal
    /// roadbed generation (with the same terrain and the plan's graph sources live) is the same
    /// roadbed and graph; a stroke that recompiled breaks the chain and the later strokes plan
    /// afresh. `None` then: the core plans and compiles the stroke itself. O(plan sources).
    pub(crate) fn rebased(
        self,
        core: &SimCore,
        live: RoadPreviewFingerprint,
    ) -> Option<(RoadEditPlan, RoadbedDelta)> {
        (live.roadbed_generation == self.fingerprint.roadbed_generation
            && live.terrain_source_generation == self.fingerprint.terrain_source_generation
            && core.transit_network.roadbed.is_current()
            && self.plan.topology_for(&core.region_graph).is_some())
        .then(|| (self.plan.rebased_to(live.surface_generation), self.delta))
    }
}

impl SimCore {
    /// Commits `request`'s strokes as one transaction (module documentation): all or none, no
    /// land taken before every stroke's verdict, one undo group, one tail. A single stroke
    /// takes the single commit's path. `road_total` starts the command's wall clock.
    pub(crate) fn commit_road_batch(
        &mut self,
        request: RoadBatchRequest,
        road_total: Instant,
        edit_metrics: RoadEditMetrics,
    ) -> RoadBatchOutcome {
        let mut strokes = request.strokes;
        if strokes.len() == 1 {
            let mut stroke = strokes.pop().expect("one stroke");
            stroke.preview = None;
            let outcome = self.commit_road(stroke, road_total, edit_metrics);
            let refused_stroke = (!outcome.committed).then_some(0);
            return RoadBatchOutcome {
                outcome,
                refused_stroke,
                times: RoadBatchTimes::default(),
            };
        }
        // The previous commit's lots first, outside the pool (`ROAD-68-G` part 7).
        self.adopt_pending_cell_lots();
        record_crash_phase_for_core(self, "add road batch shared pool wait");
        crate::thread_pools::install(|| {
            self.commit_road_batch_in_pool(strokes, road_total, edit_metrics)
        })
    }

    fn commit_road_batch_in_pool(
        &mut self,
        strokes: Vec<RoadCommitRequest>,
        road_total: Instant,
        edit_metrics: RoadEditMetrics,
    ) -> RoadBatchOutcome {
        let submitted: Vec<SubmittedStroke> = strokes.iter().map(SubmittedStroke::of).collect();
        let dry_start = Instant::now();
        record_crash_phase_for_core(self, "add road batch dry run");
        let dry = self.dry_run_batch(&strokes);
        let dry_run_ms = dry_start.elapsed().as_secs_f64() * 1000.0;
        let dry = match dry {
            Ok(dry) => dry,
            Err((index, rejection)) => {
                self.last_road_rejection = Some(rejection);
                self.last_road_timing = format!("rejected=batch stroke={index}");
                let mut batch =
                    self.finish_refused_batch(index, road_total, edit_metrics, &submitted);
                batch.times.dry_run_ms = dry_run_ms;
                return batch;
            }
        };
        // Every stroke's record joins one undo group.
        let group = self.next_undo_group();
        self.undo_batch_group = Some(group);
        let count = strokes.len();
        let mut kept_tiles = Vec::with_capacity(count);
        let mut core_ms = Vec::with_capacity(count);
        let mut reused = 0;
        let mut last = None;
        for (index, (stroke, dry)) in strokes.into_iter().zip(dry).enumerate() {
            let core_start = Instant::now();
            // The command's metrics are the last stroke's, with the command's waits.
            let metrics = if index + 1 == count {
                RoadEditMetrics {
                    queue_wait_ms: edit_metrics.queue_wait_ms,
                    lock_wait_ms: edit_metrics.lock_wait_ms,
                    ..Default::default()
                }
            } else {
                RoadEditMetrics::default()
            };
            // The last stroke's lots are planned off the commit, as a single commit's; the
            // earlier strokes' here, before the next stroke reads them.
            let mut core = self.commit_stroke_core(
                stroke,
                StrokePlan::Batch(dry),
                index + 1 == count,
                road_total,
                metrics,
            );
            core_ms.push(core_start.elapsed().as_secs_f64() * 1000.0);
            if !core.committed {
                // The dry run's verdict is the commit's, so this is unreachable; should it
                // happen, the batch's strokes so far are undone (their taken land stays
                // taken) and the batch refuses.
                debug_assert!(
                    false,
                    "batch stroke {index} refused after its dry run accepted it"
                );
                self.undo_batch_group = None;
                if index > 0 {
                    self.undo_action_in_pool();
                }
                return self.finish_refused_batch(index, road_total, core.metrics, &submitted);
            }
            reused += usize::from(core.metrics.preview_plan_reused);
            kept_tiles.extend(core.kept_tiles.take());
            last = Some(core);
        }
        self.undo_batch_group = None;
        let mut stroke = last.expect("a batch has strokes");
        let tail_start = Instant::now();
        let tail = self.commit_tail(
            true,
            stroke.deferred_lots.take(),
            kept_tiles,
            &mut stroke.metrics,
        );
        let tail_ms = tail_start.elapsed().as_secs_f64() * 1000.0;
        let times = RoadBatchTimes {
            dry_run_ms,
            core_ms,
            tail_ms,
            reused,
        };
        debug_log!("road", "road_batch strokes={count} {times:?}");
        let outcome = self.finish_commit(stroke, tail, road_total, &submitted[count - 1]);
        RoadBatchOutcome {
            outcome,
            refused_stroke: None,
            times,
        }
    }

    // A refused batch's outcome: the stroke `index` reported, the road tool's view republished
    // as after a refused single stroke.
    fn finish_refused_batch(
        &mut self,
        index: usize,
        road_total: Instant,
        mut edit_metrics: RoadEditMetrics,
        submitted: &[SubmittedStroke],
    ) -> RoadBatchOutcome {
        let tail = self.commit_tail(false, None, Vec::new(), &mut edit_metrics);
        let core = super::road_commit::StrokeCore {
            committed: false,
            deferred_lots: None,
            kept_tiles: None,
            metrics: edit_metrics,
        };
        let outcome = self.finish_commit(core, tail, road_total, &submitted[index]);
        RoadBatchOutcome {
            outcome,
            refused_stroke: Some(index),
            times: RoadBatchTimes::default(),
        }
    }

    // The batch's verdict before any land is taken (module documentation): each stroke planned,
    // checked as the commit checks it, applied to the live graph and compiled into a copy of the
    // roadbed, in order; then the graph is put back. Each stroke's plan and pieces, or the first
    // refused stroke's index and refusal. Nothing outside the graph changes, and the graph is
    // restored either way. `O(sum of the strokes' plan, local records and closure pieces)`.
    fn dry_run_batch(
        &mut self,
        strokes: &[RoadCommitRequest],
    ) -> Result<Vec<DryRunStroke>, (usize, RoadRejection)> {
        // The explicit sites are queried through the site index, as the commit's take reads it.
        self.allocator
            .prepare_building_site_query_index(self.config.zone_cell_m);
        let mut roadbed = self.transit_network.roadbed.snapshot();
        let mut checkpoints = Vec::with_capacity(strokes.len());
        let mut dry = Vec::with_capacity(strokes.len());
        let mut verdict = Ok(());
        for (index, stroke) in strokes.iter().enumerate() {
            match self.dry_run_stroke(stroke, &mut roadbed, &mut checkpoints) {
                Ok(stroke) => dry.push(stroke),
                Err(rejection) => {
                    verdict = Err((index, rejection));
                    break;
                }
            }
        }
        for checkpoint in checkpoints.iter().rev() {
            self.region_graph.restore_undo_delta_local(checkpoint);
        }
        verdict.map(|()| dry)
    }

    // One stroke of the dry run, with the commit's checks in the commit's order: the plan's
    // readiness, the stroke checks, the pieces' plotted land, then failed node pieces. Applies
    // the plan to the live graph (its checkpoint pushed to `checkpoints`) and installs the
    // stroke's pieces into `roadbed`, the dry run's copy.
    fn dry_run_stroke(
        &mut self,
        stroke: &RoadCommitRequest,
        roadbed: &mut RoadbedSystem,
        checkpoints: &mut Vec<RegionGraphUndoDelta>,
    ) -> Result<DryRunStroke, RoadRejection> {
        let fingerprint =
            RoadPreviewFingerprint::of(self.road_tool_surface_generation, roadbed, &self.heightmap);
        let plan = RoadEditPlan::compile_from(
            &self.heightmap,
            &self.region_graph,
            roadbed,
            RoadPreviewRequest {
                surface_generation: self.road_tool_surface_generation,
                points: stroke.points.clone(),
                fwd_lanes: stroke.fwd_lanes,
                bkw_lanes: stroke.bkw_lanes,
                snap_to_existing_roads: stroke.snap_to_existing_roads,
                tunnel: stroke.tunnel,
                median: stroke.median,
                parking: stroke.parking,
                attributes: stroke.attributes,
            },
        );
        // The live roadbed is the dry run's base and stays untouched, so the plan's readiness
        // reads the graph the earlier strokes left.
        if plan.status(self) != "ready" {
            return Err(plan.rejection(self));
        }
        let prepared = plan.prepared();
        let lanes = (
            stroke.fwd_lanes.clamp(0, i32::from(u8::MAX)) as u8,
            stroke.bkw_lanes.clamp(0, i32::from(u8::MAX)) as u8,
        );
        let validation = validate_road_candidate_against_water(
            prepared,
            lanes.0,
            lanes.1,
            &self.watermap,
            validate_road_candidate_fast(prepared, lanes, &self.heightmap, &self.region_graph),
        );
        if !validation.is_valid {
            return Err(RoadRejection::from_validation(
                validation.invalid_reason,
                &validation,
                &prepared.points,
            ));
        }
        let topology = plan
            .topology_for(&self.region_graph)
            .expect("a ready plan's sources are live");
        let (edges, nodes) = topology.source_topology_ids();
        let checkpoint = self
            .region_graph
            .capture_undo_for_local_topology(edges, nodes);
        let applied = topology.apply_to_graph(&mut self.region_graph);
        checkpoints.push(checkpoint);
        let applied = applied.expect("a ready plan's sources are live");
        applied.mark_pieces(roadbed);
        let delta = roadbed.compile_delta(&self.region_graph);
        let (_, restore) = roadbed.install(&delta);
        let before: Vec<_> = restore.replaced_pieces().collect();
        let installed: Vec<_> = roadbed.installed_pieces(&restore).collect();
        if let Some(overlap) = RoadLandTake::in_core(self, &installed, &before).plotted {
            return Err(overlap.rejection(self));
        }
        if let Some(&node) = roadbed.failed_nodes().first() {
            let reason = format!(
                "node_piece_failed node={node} error={:?}",
                roadbed.node_error(node)
            );
            return Err(RoadRejection::new(
                "render_geometry_invalid",
                format!(
                    "the road surface failed to build after the edit ({reason}); the edit was \
                     rolled back"
                ),
            ));
        }
        Ok(DryRunStroke {
            plan,
            delta,
            fingerprint,
        })
    }
}
