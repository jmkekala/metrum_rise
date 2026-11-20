// SPDX-License-Identifier: GPL-3.0-or-later

//! Authoritative road placement: plan reuse, transactional commit and dependent rebuilds.
//!
//! The sim thread runs this for every `SimCommand::AddRoad`; headless scenarios call it to lay
//! roads through the same path as the player.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use super::frame::record_crash_phase_for_core;
use super::lot_adoption::CommitCellLotInputs;
use super::road_edit_plan::RoadEditPlan;
use super::road_preview::{RoadPreview, RoadPreviewFingerprint, RoadPreviewRequest};
use super::road_rejection::{RejectedBy, RoadAttempt, RoadRejection};
use super::road_tool_context::RoadToolPublication;
use super::state::SimCore;
use crate::debug_log;
use crate::nodes::sim::benchmark::road_edit::RoadEditMetrics;
use crate::simulation::core::pure_job::JobStart;
use crate::simulation::network::road_edit::{FinalizedRoadGeometry, RoadProfile};
use crate::simulation::network::road_types::RoadTypePockets;
use crate::simulation::network::roadbed::RoadbedDelta;
use crate::simulation::zoning::cells::CellBounds;
use godot::prelude::Vector3;

/// One player road stroke as submitted by the road tool.
pub(crate) struct RoadCommitRequest {
    /// World-space polyline points.
    pub(crate) points: Vec<Vector3>,
    /// Forward lane count.
    pub(crate) fwd_lanes: i32,
    /// Backward lane count.
    pub(crate) bkw_lanes: i32,
    /// Whether authored endpoints may snap to nearby existing road nodes.
    pub(crate) snap_to_existing_roads: bool,
    /// How the stroke's heights are made (`ROAD-112` round H; the tunnel intent of `ROAD-61` is
    /// `RoadProfile::Tunnel`): a preview made with another is not this stroke's, and a fresh
    /// plan prepares the stroke with it.
    pub(crate) profile: RoadProfile,
    /// The road tool's median (`ROAD-80`): a preview made without it is not this stroke's, and
    /// the stroke's edge carries it.
    pub(crate) median: Option<crate::simulation::network::graph::RoadMedian>,
    /// The road tool's kerbside parking (`PARK-05`), carried like `median`.
    pub(crate) parking: crate::simulation::network::graph::RoadParking,
    /// The road tool's road-type attributes (`ROAD-101`): a preview made with others is not this
    /// stroke's, and the stroke's edge and pieces carry them.
    pub(crate) attributes: crate::simulation::network::road_types::RoadAttributes,
    /// The road tool's preview of this stroke (`ROAD-68-E`): its plan and pieces are installed
    /// when its fingerprint is still current, else the commit recompiles locally.
    pub(crate) preview: Option<std::sync::Arc<RoadPreview>>,
}

/// What the road tool learns about one queued stroke.
pub(crate) struct RoadCommitAck {
    /// Whether the road was accepted.
    pub(crate) committed: bool,
    /// Stable rejection reason key, as in the `events` report; empty when committed.
    pub(crate) reason: String,
    /// Rejection diagnostic; empty when committed.
    pub(crate) detail: String,
    /// The road-tool surface generation after the command: the render generation whose road
    /// chunks replace the preview's (`ROAD-68-E`).
    pub(crate) generation: u64,
    /// The stroke that refused a batch (`ROAD-112` round F); `None` for a single stroke or a
    /// committed batch.
    pub(crate) refused_stroke: Option<usize>,
    /// Refused optional pockets, with the junction and approach that could not receive them.
    pub(crate) pocket_refusals: Vec<StrokePocketRefusal>,
}

/// One optional approach pocket refused after the road itself committed.
pub(crate) struct StrokePocketRefusal {
    /// Junction position for player feedback, independent of edge splitting.
    pub(crate) position: Vector3,
    /// Original stroke edge and junction node identify the approach.
    pub(crate) edge: usize,
    /// Junction id.
    pub(crate) node: u32,
    /// Approach bearing clockwise from north, matching the Junction window.
    pub(crate) bearing_deg: f32,
    /// Existing pocket refusal key, translated by RoadReasonText.
    pub(crate) reason: String,
}

/// Result and phase timings of [`SimCore::commit_road`].
pub(crate) struct RoadCommitOutcome {
    /// Whether the road was accepted.
    pub(crate) committed: bool,
    /// Stable rejection reason key; empty when committed.
    pub(crate) rejection_reason: String,
    /// Rejection diagnostic; empty when committed.
    pub(crate) rejection: String,
    /// Road-tool context publication for the committed generation.
    pub(crate) road_snapshot: Option<RoadToolPublication>,
    pub(crate) add_internal_ms: f64,
    pub(crate) finalize_ms: f64,
    pub(crate) surface_ms: f64,
    pub(crate) mesh_ms: f64,
    pub(crate) snapshot_ms: f64,
    /// The road-tool surface generation after the command.
    pub(crate) generation: u64,
    /// Refused optional pockets, with the junction and approach that could not receive them.
    pub(crate) pocket_refusals: Vec<StrokePocketRefusal>,
}

impl SimCore {
    /// Validates and commits one road stroke, then refreshes lanes, agents, entrances, routing,
    /// road meshes and the graded terrain. A rejected stroke leaves the network unchanged and is
    /// reported on `events` with its exact inputs.
    ///
    /// `road_total` starts the command's wall-clock measurement; `edit_metrics` arrives with
    /// queue and lock waits filled in and is stored as `last_road_edit_metrics`.
    pub(crate) fn commit_road(
        &mut self,
        request: RoadCommitRequest,
        road_total: Instant,
        edit_metrics: RoadEditMetrics,
    ) -> RoadCommitOutcome {
        self.commit_road_with_pockets(
            request,
            RoadTypePockets::default(),
            road_total,
            edit_metrics,
        )
    }

    /// Commits a stroke and requests the selected type's pockets only on its new approaches.
    pub(crate) fn commit_road_with_pockets(
        &mut self,
        request: RoadCommitRequest,
        pockets: RoadTypePockets,
        road_total: Instant,
        edit_metrics: RoadEditMetrics,
    ) -> RoadCommitOutcome {
        // The previous commit's lots first, outside the pool (`ROAD-68-G` part 7): this edit
        // reads and marks them.
        self.adopt_pending_cell_lots();
        // The commit's rebuilds are parallel: shared pool (CODE-17).
        record_crash_phase_for_core(self, "add road shared pool wait");
        crate::thread_pools::install(|| {
            record_crash_phase_for_core(self, "add road shared pool entered");
            self.commit_road_in_pool(request, pockets, road_total, edit_metrics)
        })
    }

    fn commit_road_in_pool(
        &mut self,
        request: RoadCommitRequest,
        pockets: RoadTypePockets,
        road_total: Instant,
        edit_metrics: RoadEditMetrics,
    ) -> RoadCommitOutcome {
        let preview = request.preview.clone();
        let submitted = SubmittedStroke {
            pockets,
            ..SubmittedStroke::of(&request)
        };
        let mut stroke = self.commit_stroke_core(
            request,
            StrokePlan::Preview(preview),
            pockets,
            true,
            road_total,
            edit_metrics,
        );
        let tail = self.commit_tail(
            stroke.committed,
            stroke.deferred_lots.take(),
            stroke.kept_tiles.take().into_iter().collect(),
            &mut stroke.metrics,
        );
        self.finish_commit(stroke, tail, road_total, &submitted)
    }

    // The per-stroke core of a commit (`ROAD-112` round F, plan 6.1): plan (or the preview's or
    // the batch's), add, finalize geometry, install pieces, take land, local lanes and agents,
    // entrances, and the charge. With `defer_lots` the lot regions the edit queued are left for
    // the tail's deferred plan; otherwise they are planned here, with the entrances that follow
    // them, which a deferred plan's adoption equals bitwise (`ROAD-68-G` part 7). A refused
    // stroke leaves the network unchanged. `O(the stroke's local work)`.
    pub(super) fn commit_stroke_core(
        &mut self,
        request: RoadCommitRequest,
        plan: StrokePlan,
        pockets: RoadTypePockets,
        defer_lots: bool,
        road_total: Instant,
        mut edit_metrics: RoadEditMetrics,
    ) -> StrokeCore {
        let RoadCommitRequest {
            points,
            fwd_lanes,
            bkw_lanes,
            snap_to_existing_roads,
            profile,
            median,
            parking,
            attributes,
            preview: _,
        } = request;
        self.last_road_rejection = None;
        // Bulk-load defers per-edge rebuilds until finalization.
        let add_internal_start = Instant::now();
        // Complete the local road plan before opening the transaction; it never clones the city.
        // A preview of exactly this stroke on the live generation is the commit's plan, and its
        // pieces are what the commit installs (`ROAD-68-E`); so is a batch's dry run of it while
        // the batch's chain holds (`ROAD-112` round F); otherwise plan here.
        let live_fingerprint = RoadPreviewFingerprint::live(self);
        let (reused_plan, reused_pieces) = match plan {
            StrokePlan::Preview(preview) => {
                let preview = preview.filter(|preview| {
                    preview.pockets == pockets
                        && preview.fingerprint == live_fingerprint
                        && preview.is_for(
                            &points,
                            fwd_lanes,
                            bkw_lanes,
                            snap_to_existing_roads,
                            profile,
                            (median, parking, attributes),
                        )
                });
                (
                    preview.as_ref().map(|preview| PlanSource::Preview(Arc::clone(preview))),
                    preview.map(PiecesSource::Preview),
                )
            }
            StrokePlan::Batch(dry) => match (*dry).rebased(self, live_fingerprint) {
                Some((plan, delta)) => (
                    Some(PlanSource::Owned(Box::new(plan))),
                    Some(PiecesSource::Delta(delta)),
                ),
                None => (None, None),
            },
        };
        edit_metrics.preview_plan_reused = reused_plan.is_some();
        record_crash_phase_for_core(self, "add road plan compile");
        let fresh_plan;
        let edit_plan = match &reused_plan {
            Some(PlanSource::Preview(preview)) => &preview.plan,
            Some(PlanSource::Owned(plan)) => &**plan,
            None => {
                fresh_plan = RoadEditPlan::compile(
                    self,
                    RoadPreviewRequest {
                        surface_generation: self.road_tool_surface_generation,
                        points: points.clone(),
                        fwd_lanes,
                        bkw_lanes,
                        snap_to_existing_roads,
                        profile,
                        median,
                        parking,
                        attributes,
                    },
                );
                &fresh_plan
            }
        };
        record_crash_phase_for_core(self, "add road plan compiled");
        edit_metrics.road_plan_ms = add_internal_start.elapsed().as_secs_f64() * 1000.0;
        record_crash_phase_for_core(self, "add road transaction start");
        self.transit_network.bulk_load = true;
        self.transit_network.begin_road_edit();
        record_crash_phase_for_core(self, "add road transaction started");
        record_crash_phase_for_core(self, "add road internal");
        let mut road_add = self.add_road_internal_with_snap_and_validation(
            points,
            fwd_lanes,
            bkw_lanes,
            snap_to_existing_roads,
            Some(edit_plan),
        );
        let add_internal_ms = add_internal_start.elapsed().as_secs_f64() * 1000.0;
        edit_metrics.undo_ms = road_add.stage_ms.undo_ms;
        edit_metrics.topology_ms = road_add.stage_ms.topology_ms;
        edit_metrics.zoning_ms = road_add.stage_ms.zoning_ms;
        let finalize_start = Instant::now();
        let mut surface_ms = 0.0;
        // The lot regions the edit queued and its dirty edges, planned after the publication.
        let mut deferred_lots = None;
        let mut pocket_refusals = Vec::new();
        if road_add.committed {
            self.transit_network.bulk_load = false;
            record_crash_phase_for_core(self, "add road geometry finalize");

            let FinalizedRoadGeometry {
                dirty_edges: mut dirty,
                stroke_edges,
                affected_nodes: _affected_nodes,
                merged_nodes,
                profile_us: dt_profile_us,
                clips_us: dt_clips_us,
                ..
            } = road_add.finalized_geometry.take().unwrap_or_else(|| {
                let mut geometry = self.finalize_bulk_road_geometry_for_dirty_edges();
                geometry.stroke_edges = std::mem::take(&mut road_add.stroke_edges);
                geometry
            });
            let roadbed_before =
                (pockets.left || pockets.right).then(|| self.transit_network.roadbed.snapshot());
            let surface_start = Instant::now();
            record_crash_phase_for_core(self, "add road render validation");
            // A batch's dry run judged its pieces' plotted land (`ROAD-112` round F2). A pocket
            // preview shows the complete stroke: the base transaction installs only its base
            // pieces, and accepted pockets follow through the shared checks below.
            let (delta, graded, plotted_judged) = match &reused_pieces {
                _ if roadbed_before.is_some() => (None, None, false),
                Some(PiecesSource::Preview(preview)) => match preview.pieces.as_ref() {
                    Some(pieces) => (Some(&pieces.delta), Some(&pieces.graded), false),
                    None => (None, None, false),
                },
                Some(PiecesSource::Delta(delta)) => (Some(delta), None, true),
                None => (None, None, false),
            };
            road_add.committed = self.validate_staged_road_render_installing(
                delta,
                graded,
                &road_add.regeneration,
                plotted_judged,
                &mut edit_metrics,
                roadbed_before.is_none(),
            );
            surface_ms = surface_start.elapsed().as_secs_f64() * 1000.0;
            if road_add.committed {
                pocket_refusals = self.apply_stroke_pockets(&stroke_edges, pockets, &mut dirty);
                if let Some(before) = roadbed_before {
                    let graded_start = Instant::now();
                    let preview_graded = match &reused_pieces {
                        Some(PiecesSource::Preview(preview)) => {
                            preview.pieces.as_ref().map(|pieces| &pieces.graded)
                        }
                        _ => None,
                    };
                    let graded = self.stage_committed_graded_tiles(before, preview_graded);
                    if !self.benchmark_mode
                        && let Some(record) = self.undo_stack.back_mut()
                    {
                        record.graded = graded;
                    }
                    edit_metrics.surface_graded_ms = graded_start.elapsed().as_secs_f64() * 1000.0;
                    let stats = self.last_graded_stats;
                    edit_metrics.graded_select_ms = stats.select_ms;
                    edit_metrics.graded_bake_ms = stats.bake_ms;
                    edit_metrics.graded_adopted = stats.adopted;
                    edit_metrics.graded_baked = stats.baked;
                }
                surface_ms = surface_start.elapsed().as_secs_f64() * 1000.0;
                let dirty_count = dirty.len();
                edit_metrics.dirty_edges = dirty_count;
                let t_inv = Instant::now();
                // Invalidate agents BEFORE lane rebuild so old lane IDs are still valid.
                record_crash_phase_for_core(self, "add road lane invalidation");
                self.agents.invalidate_lane_ids_for_edges(
                    &dirty,
                    &self.transit_network.lane_system,
                    &self.region_graph,
                );
                let dt_inv_us = t_inv.elapsed().as_micros();
                edit_metrics.agent_invalidate_ms = dt_inv_us as f64 / 1000.0;

                let t_lanes = Instant::now();
                record_crash_phase_for_core(self, "add road lane rebuild");
                // A merged node's connectors (a dissolved joint's) go with it (`ROAD-125`).
                self.transit_network
                    .lane_system
                    .retire_node_lanes(merged_nodes.iter().map(|&node| node as usize));
                self.transit_network
                    .lane_system
                    .rebuild_edges_incremental(&mut self.region_graph, &dirty);
                self.agents.reattach_invalidated_lanes_for_edges(
                    &dirty,
                    &self.transit_network.lane_system,
                    &self.region_graph,
                );
                // The rebuilt lanes stand on the roadbed the commit installed before the
                // entrances sample their kerbs on them, here or at the lot plan's adoption
                // (`PARK-16`); the tail's mesh precompute then finds them lifted.
                self.transit_network
                    .sync_new_lane_heights(&self.region_graph);
                // A border the commit cut off or merged settles its freight (`SAVE-06`).
                self.settle_freight_of_unreachable_borders();
                let dt_lanes_us = t_lanes.elapsed().as_micros();
                edit_metrics.lanes_and_reattach_ms = dt_lanes_us as f64 / 1000.0;
                let buildings_start = Instant::now();
                record_crash_phase_for_core(self, "add road entrance rebuild");
                // Lots the edit queued are planned off the commit and adopted, with the
                // entrances that follow them, at the next frame (question 33 (c)); a batch's
                // earlier strokes plan theirs here, before the next stroke reads them.
                let regions = if defer_lots {
                    self.take_commit_cell_lot_regions()
                } else {
                    Vec::new()
                };
                if regions.is_empty() {
                    self.rebuild_building_entrances_for_commit(&dirty);
                } else {
                    deferred_lots = Some((regions, dirty));
                }
                edit_metrics.buildings_ms = buildings_start.elapsed().as_secs_f64() * 1000.0;

                // The routing hierarchy is rebuilt off the commit once the road-tool snapshot
                // exists, from its graph copy (D7, below).

                // Cell-lot preparation consumes queued painted neighborhoods at
                // the next hourly demand pass; road placement does not scan cells.

                let total_us = road_total.elapsed().as_micros();
                let msg = format!(
                    "TOTAL={}µs  {}  profiles={}µs  clips={}µs  lanes={}µs({}e)  invalidate={}µs",
                    total_us,
                    self.last_road_timing,
                    dt_profile_us,
                    dt_clips_us,
                    dt_lanes_us,
                    dirty_count,
                    dt_inv_us
                );
                debug_log!("road", "{}", msg);
                self.last_road_timing = msg;
                // The record keeps the charge, which undo refunds (U5).
                self.charge_undoable_build_cost(road_add.build_cost);
            }
        } else {
            self.transit_network.bulk_load = false;
            self.transit_network.accept_road_edit();
        }
        let finalize_ms = (finalize_start.elapsed().as_secs_f64() * 1000.0 - surface_ms).max(0.0);
        // The undo record's kept terrain tiles (`TERRAIN-07-E`), queued by the tail.
        let kept_tiles = if road_add.committed {
            self.take_kept_graded_tiles_start()
        } else {
            None
        };
        edit_metrics.add_ms = add_internal_ms;
        edit_metrics.finalize_ms = finalize_ms;
        edit_metrics.surface_ms = surface_ms;
        StrokeCore {
            committed: road_add.committed,
            pocket_refusals,
            deferred_lots,
            kept_tiles,
            metrics: edit_metrics,
        }
    }

    // The once-per-command tail (`ROAD-112` round F, plan 6.1): mesh precompute, the road tool
    // view's publication with the commit's one `O(city)` graph copy, the routing rebuild's
    // schedule, the deferred lot plan's spawn and the kept graded tiles' jobs. A batch pays it
    // once for all its strokes.
    pub(super) fn commit_tail(
        &mut self,
        committed: bool,
        deferred_lots: Option<(Vec<CellBounds>, HashSet<usize>)>,
        kept_tiles: Vec<JobStart>,
        edit_metrics: &mut RoadEditMetrics,
    ) -> CommitTail {
        let mesh_start = Instant::now();
        record_crash_phase_for_core(self, "add road mesh precompute");
        self.precompute_road_mesh_data();
        let mesh_ms = mesh_start.elapsed().as_secs_f64() * 1000.0;
        let snapshot_start = Instant::now();
        record_crash_phase_for_core(self, "add road tool snapshot");
        // D7 (`ROAD-68-G`): the hierarchy is rebuilt as a pure job on an immutable copy of the
        // graph and adopted a fixed number of frames later; route readers wait for it. That
        // copy is the commit's one `O(city)` copy, made by the publication; the road tool's
        // view shares it and its replica copies only the rebuilt owners' records (`ROAD-68-E`).
        let (routing_graph, road_snapshot) = if committed {
            let (graph, publication) = self.publish_committed_road_tool_view();
            (Some(graph), publication)
        } else {
            (None, self.publish_road_tool_view())
        };
        let snapshot_ms = snapshot_start.elapsed().as_secs_f64() * 1000.0;
        // The background jobs start after the commit's last stage (`ROAD-68-G` part 7), so they
        // take none of its workers' cores. The routing rebuild starts when the lot plan ends
        // (part 8), so it does not share the background pool with the plan, which the next
        // frame adopts; the rebuild's adoption frame comes later. The undo records' kept
        // terrain tiles (`TERRAIN-07-E`) are queued last (part 10).
        if let Some(graph) = routing_graph {
            record_crash_phase_for_core(self, "add road cch rebuild scheduled");
            let routing_start = Instant::now();
            let routing = self
                .transit_network
                .schedule_cch_rebuild_unstarted(Arc::clone(&graph));
            edit_metrics.routing_ms = routing_start.elapsed().as_secs_f64() * 1000.0;
            match deferred_lots {
                Some((regions, dirty)) => {
                    record_crash_phase_for_core(self, "add road cell lots deferred");
                    let lots_start = Instant::now();
                    let view = road_snapshot
                        .as_ref()
                        .map(|publication| (&publication.view.roadbed, &publication.view.cells));
                    let inputs = CommitCellLotInputs {
                        graph: &graph,
                        view,
                    };
                    self.spawn_commit_cell_lots(regions, dirty, inputs, routing);
                    edit_metrics.lots_spawn_ms = lots_start.elapsed().as_secs_f64() * 1000.0;
                }
                None => routing.start(),
            }
        }
        for kept in kept_tiles {
            kept.start();
        }
        CommitTail {
            road_snapshot,
            mesh_ms,
            snapshot_ms,
        }
    }

    // The command's outcome and metrics after its tail; a refused stroke is reported on `events`
    // with its exact inputs.
    pub(super) fn finish_commit(
        &mut self,
        stroke: StrokeCore,
        tail: CommitTail,
        road_total: Instant,
        submitted: &SubmittedStroke,
    ) -> RoadCommitOutcome {
        let StrokeCore {
            committed,
            pocket_refusals,
            metrics: mut edit_metrics,
            ..
        } = stroke;
        let add_internal_ms = edit_metrics.add_ms;
        let finalize_ms = edit_metrics.finalize_ms;
        let surface_ms = edit_metrics.surface_ms;
        edit_metrics.generation = self.road_tool_surface_generation;
        edit_metrics.committed = committed;
        edit_metrics.core_work_ms =
            road_total.elapsed().as_secs_f64() * 1000.0 - edit_metrics.lock_wait_ms;
        edit_metrics.mesh_ms = tail.mesh_ms;
        edit_metrics.snapshot_ms = tail.snapshot_ms;
        self.last_road_edit_metrics = edit_metrics;
        let rejection_reason = if committed {
            String::new()
        } else {
            self.report_refused_stroke(submitted)
        };
        RoadCommitOutcome {
            committed,
            rejection_reason,
            rejection: if committed {
                String::new()
            } else {
                self.last_road_timing.clone()
            },
            road_snapshot: tail.road_snapshot,
            add_internal_ms,
            finalize_ms,
            surface_ms,
            mesh_ms: tail.mesh_ms,
            snapshot_ms: tail.snapshot_ms,
            generation: self.road_tool_surface_generation,
            pocket_refusals,
        }
    }

    // Reports the refusal `last_road_rejection` holds for `submitted` on `events` and returns
    // its reason key.
    pub(super) fn report_refused_stroke(&mut self, submitted: &SubmittedStroke) -> String {
        let rejection = self
            .last_road_rejection
            .take()
            .unwrap_or_else(|| RoadRejection::new("unknown", self.last_road_timing.clone()));
        rejection.report(
            RejectedBy::Commit,
            submitted.attempt(),
            self.road_tool_surface_generation,
        );
        rejection.reason
    }
}

/// Where a stroke's core takes its plan and pieces from.
pub(super) enum StrokePlan {
    /// The road tool's preview of the stroke, used when its fingerprint is current; else (or
    /// `None`) the core plans here.
    Preview(Option<Arc<RoadPreview>>),
    /// The batch's dry run of the stroke (`ROAD-112` round F), used while the batch's chain
    /// holds ([`super::road_batch::DryRunStroke::rebased`]).
    Batch(Box<super::road_batch::DryRunStroke>),
}

// The plan a core reuses, and the pieces it installs.
enum PlanSource {
    Preview(Arc<RoadPreview>),
    Owned(Box<RoadEditPlan>),
}

enum PiecesSource {
    Preview(Arc<RoadPreview>),
    Delta(RoadbedDelta),
}

/// What one stroke's core leaves to the command's tail.
pub(super) struct StrokeCore {
    /// Whether the stroke was accepted.
    pub(super) committed: bool,
    /// Refused optional pockets of an accepted stroke.
    pub(super) pocket_refusals: Vec<StrokePocketRefusal>,
    /// The lot regions the stroke queued and its dirty edges, for the tail's deferred plan.
    pub(super) deferred_lots: Option<(Vec<CellBounds>, HashSet<usize>)>,
    /// The undo record's kept graded tiles' job, which the tail starts.
    pub(super) kept_tiles: Option<JobStart>,
    /// The stroke's stage times.
    pub(super) metrics: RoadEditMetrics,
}

/// What the tail published, and its times.
pub(super) struct CommitTail {
    /// Road-tool context publication for the committed generation.
    pub(super) road_snapshot: Option<RoadToolPublication>,
    pub(super) mesh_ms: f64,
    pub(super) snapshot_ms: f64,
}

/// A stroke as submitted, for the refusal report.
pub(super) struct SubmittedStroke {
    points: Vec<Vector3>,
    fwd_lanes: i32,
    bkw_lanes: i32,
    snap: bool,
    profile: RoadProfile,
    median: Option<crate::simulation::network::graph::RoadMedian>,
    parking: crate::simulation::network::graph::RoadParking,
    attributes: crate::simulation::network::road_types::RoadAttributes,
    // The road type's pockets the stroke asked for (`ROAD-128`), which its replay requests.
    pockets: RoadTypePockets,
}

impl SubmittedStroke {
    /// The stroke exactly as `request` submits it, without pockets: a batch's strokes take none
    /// (a single stroke's commit sets its own).
    pub(super) fn of(request: &RoadCommitRequest) -> Self {
        Self {
            points: request.points.clone(),
            fwd_lanes: request.fwd_lanes,
            bkw_lanes: request.bkw_lanes,
            snap: request.snap_to_existing_roads,
            profile: request.profile,
            median: request.median,
            parking: request.parking,
            attributes: request.attributes,
            pockets: RoadTypePockets::default(),
        }
    }

    fn attempt(&self) -> RoadAttempt<'_> {
        RoadAttempt {
            points: &self.points,
            fwd_lanes: self.fwd_lanes,
            bkw_lanes: self.bkw_lanes,
            snap: self.snap,
            profile: self.profile,
            median: self.median,
            parking: self.parking,
            attributes: self.attributes,
            pockets: self.pockets,
        }
    }
}

impl SimCore {
    // O(stroke edges * endpoint degree + sum(pocket closure work)). Stable edge/end order.
    // Endpoints are captured before splitting; later requests resolve through only that node's
    // incidence, so a pocket at the other end cannot redirect the request to its transition.
    // `edges` are the stroke's own live edges after its plan's splits and dissolves
    // (`AppliedRoadTopology::stroke_edges`): a piece merged into a resident road is not one, so
    // its approach asks for no pocket (decision D2 of the icon UI merge).
    fn apply_stroke_pockets(
        &mut self,
        edges: &[usize],
        pockets: RoadTypePockets,
        dirty: &mut HashSet<usize>,
    ) -> Vec<StrokePocketRefusal> {
        use crate::simulation::network::pockets::{PocketEndView, StrokePocketTargets};
        let mut targets = StrokePocketTargets::new(&self.region_graph, edges, pockets);
        if targets.is_empty() {
            return Vec::new();
        }
        // Finish the plain stroke's paint inheritance before a pocket opens another erase.
        self.plan_lots_after_erase();
        // The stroke's record, on top: its commit pushed it (benchmark mode dropped it on
        // acceptance). Each pocket's record is taken off as made, so no pocket's push can retire
        // an older undo group, and all fold into the stroke's below.
        let mut records = Vec::new();
        if !self.benchmark_mode {
            records.extend(self.undo_stack.pop_back());
        }
        let mut refused = Vec::new();
        let mut terrain = self.last_road_terrain_changes.clone();
        while let Some((original, node, request)) = targets.next(&self.region_graph) {
            let before = self.region_graph.edge_count();
            let depth = self.undo_stack.len();
            if self.set_stroke_pocket_internal(request, dirty) {
                targets.record(original, before..self.region_graph.edge_count());
                let changed = &self.last_road_terrain_changes;
                terrain.pieces.extend_from_slice(&changed.pieces);
                terrain.chains.extend_from_slice(&changed.chains);
                terrain.segments.extend_from_slice(&changed.segments);
                terrain.portals.extend_from_slice(&changed.portals);
                // The pocket's record, with its charge (`push_pocket_undo`).
                let record = if self.undo_stack.len() > depth {
                    self.undo_stack.pop_back()
                } else {
                    None
                };
                if !self.benchmark_mode {
                    records.extend(record);
                }
            } else {
                // A refused edit changed nothing and left its reason (`apply_road_pocket`).
                let reason = self
                    .last_road_rejection
                    .take()
                    .map_or_else(|| "unknown".to_owned(), |rejection| rejection.reason);
                let bearing_deg = PocketEndView::of(&self.region_graph, request.edge, request.end)
                    .map_or(0.0, |view| crate::nodes::sim::road_pockets::bearing_out(&view.road));
                refused.push(StrokePocketRefusal {
                    bearing_deg,
                    position: self.region_graph.node(node).pos,
                    edge: original,
                    node,
                    reason,
                });
            }
        }
        self.last_road_terrain_changes = terrain;
        if !records.is_empty() {
            self.push_stroke_pocket_undo(records);
        }
        refused
    }

    // `rebuild_building_entrances_internal` for a commit that queued no lot region
    // (`ROAD-68-G`): the same lot refresh and attachment repair, but only the entrances whose
    // inputs the edit may have changed are derived again (`repair_commit_entrances`).
    fn rebuild_building_entrances_for_commit(&mut self, dirty: &HashSet<usize>) {
        crate::thread_pools::install(|| {
            let cache_current = self.entrance_cache_current();
            let lots = self.prepare_cell_lots_internal();
            self.repair_commit_entrances(dirty, cache_current, &lots, &[]);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debug::log;
    use crate::simulation::core::config::WorldConfig;

    #[test]
    fn refused_commit_reports_reason_place_and_exact_stroke() {
        let mut core = SimCore::new(WorldConfig::default());
        let before = log::read_since(0, 0, |_| {});
        // Half a metre is below the minimum road length.
        let points = vec![
            Vector3::new(123.25, 0.0, -456.5),
            Vector3::new(123.75, 0.0, -456.5),
        ];
        let outcome = core.commit_road(
            RoadCommitRequest {
                points: points.clone(),
                fwd_lanes: 2,
                bkw_lanes: 1,
                snap_to_existing_roads: false,
                profile: RoadProfile::FollowGround,
                median: None,
                parking: crate::simulation::network::graph::RoadParking::NONE,
                attributes: crate::simulation::network::road_types::RoadAttributes::DEFAULT,
                preview: None,
            },
            Instant::now(),
            Default::default(),
        );
        assert!(!outcome.committed);
        assert_eq!(outcome.rejection_reason, "too_short");
        assert!(
            core.last_road_rejection.is_none(),
            "the report consumes the rejection"
        );

        // Other tests log concurrently; keep only this stroke's lines.
        let mut lines = Vec::new();
        log::read_since(before, usize::MAX, |line| {
            if line.channel == crate::debug::Channel::Events && line.message.contains("123.") {
                lines.push(line.message.clone());
            }
        });
        let why = lines
            .iter()
            .find(|line| line.starts_with("road rejected by commit: too_short"))
            .expect("events explains the refusal");
        assert!(why.contains("Stroke (123.2, -456.5) -> (123.8, -456.5), 2 points, 0.5 m"));
        assert!(why.contains("lanes 2/1, snap off"));
        let replay = lines
            .iter()
            .find_map(|line| line.strip_prefix("road_attempt "))
            .expect("events records the attempt");
        let record: serde_json::Value = serde_json::from_str(replay).unwrap();
        assert_eq!(record["by"], "commit");
        assert_eq!(record["reason"], "too_short");
        assert_eq!(record["generation"], core.road_tool_surface_generation);
        assert!(why.ends_with(&format!("Attempt {}.", record["attempt"])));
        let replayed: Vec<[f32; 3]> = serde_json::from_value(record["points"].clone()).unwrap();
        assert_eq!(
            replayed,
            points.iter().map(|p| [p.x, p.y, p.z]).collect::<Vec<_>>()
        );
    }
}
