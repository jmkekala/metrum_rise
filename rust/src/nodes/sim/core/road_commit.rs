// SPDX-License-Identifier: GPL-2.0-only

//! Authoritative road placement: plan reuse, transactional commit and dependent rebuilds.
//!
//! The sim thread runs this for every `SimCommand::AddRoad`; headless scenarios call it to lay
//! roads through the same path as the player.

use std::collections::HashSet;
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
use crate::simulation::network::road_edit::FinalizedRoadGeometry;
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
}

impl SimCore {
    /// Validates and commits one road stroke, then refreshes lanes, agents, entrances, routing,
    /// road meshes and road-locked terrain. A rejected stroke leaves the network unchanged and is
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
        // The previous commit's lots first, outside the pool (`ROAD-68-G` part 7): this edit
        // reads and marks them.
        self.adopt_pending_cell_lots();
        // The commit's rebuilds are parallel: shared pool (CODE-17).
        record_crash_phase_for_core(self, "add road shared pool wait");
        crate::thread_pools::install(|| {
            record_crash_phase_for_core(self, "add road shared pool entered");
            self.commit_road_in_pool(request, road_total, edit_metrics)
        })
    }

    fn commit_road_in_pool(
        &mut self,
        request: RoadCommitRequest,
        road_total: Instant,
        mut edit_metrics: RoadEditMetrics,
    ) -> RoadCommitOutcome {
        let RoadCommitRequest {
            points,
            fwd_lanes,
            bkw_lanes,
            snap_to_existing_roads,
            preview,
        } = request;
        self.last_road_rejection = None;
        // The report of a refusal carries the stroke exactly as submitted.
        let submitted_points = points.clone();
        // Bulk-load defers per-edge rebuilds until finalization.
        let add_internal_start = Instant::now();
        // Complete the local road plan before opening the transaction; it never clones the city.
        // A preview of exactly this stroke on the live generation is the commit's plan, and its
        // pieces are what the commit installs (`ROAD-68-E`); otherwise plan here.
        let live_fingerprint = RoadPreviewFingerprint::live(self);
        let preview = preview.filter(|preview| {
            preview.fingerprint == live_fingerprint
                && preview.is_for(&points, fwd_lanes, bkw_lanes, snap_to_existing_roads)
        });
        edit_metrics.preview_plan_reused = preview.is_some();
        record_crash_phase_for_core(self, "add road plan compile");
        let fresh_plan;
        let edit_plan = match &preview {
            Some(preview) => &preview.plan,
            None => {
                fresh_plan = RoadEditPlan::compile(
                    self,
                    RoadPreviewRequest {
                        surface_generation: self.road_tool_surface_generation,
                        points: points.clone(),
                        fwd_lanes,
                        bkw_lanes,
                        snap_to_existing_roads,
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
        if road_add.committed {
            self.transit_network.bulk_load = false;
            record_crash_phase_for_core(self, "add road geometry finalize");

            let FinalizedRoadGeometry {
                dirty_edges: dirty,
                affected_nodes: _affected_nodes,
                profile_us: dt_profile_us,
                clips_us: dt_clips_us,
                ..
            } = road_add
                .finalized_geometry
                .take()
                .unwrap_or_else(|| self.finalize_bulk_road_geometry_for_dirty_edges());
            let dirty_count = dirty.len();
            edit_metrics.dirty_edges = dirty_count;
            let surface_start = Instant::now();
            record_crash_phase_for_core(self, "add road render validation");
            let preview_pieces = preview.as_ref().and_then(|preview| preview.pieces.as_ref());
            road_add.committed = self.validate_staged_road_render_installing(
                preview_pieces.map(|pieces| &pieces.delta),
                preview_pieces.map(|pieces| &pieces.graded),
                &mut edit_metrics,
            );
            surface_ms = surface_start.elapsed().as_secs_f64() * 1000.0;
            if road_add.committed {
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
                self.transit_network
                    .lane_system
                    .rebuild_edges_incremental(&mut self.region_graph, &dirty);
                self.agents.reattach_invalidated_lanes_for_edges(
                    &dirty,
                    &self.transit_network.lane_system,
                    &self.region_graph,
                );
                let dt_lanes_us = t_lanes.elapsed().as_micros();
                edit_metrics.lanes_and_reattach_ms = dt_lanes_us as f64 / 1000.0;
                let buildings_start = Instant::now();
                record_crash_phase_for_core(self, "add road entrance rebuild");
                // Lots the edit queued are planned off the commit and adopted, with the
                // entrances that follow them, at the next frame (question 33 (c)).
                let regions = self.take_commit_cell_lot_regions();
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
                if !self.benchmark_mode {
                    self.treasury.deduct_build_cost(road_add.build_cost);
                }
            }
        } else {
            self.transit_network.bulk_load = false;
            self.transit_network.accept_road_edit();
        }
        let finalize_ms = (finalize_start.elapsed().as_secs_f64() * 1000.0 - surface_ms).max(0.0);
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
        let (routing_graph, road_snapshot) = if road_add.committed {
            let (graph, publication) = self.publish_committed_road_tool_view();
            (Some(graph), publication)
        } else {
            (None, self.publish_road_tool_view())
        };
        let snapshot_ms = snapshot_start.elapsed().as_secs_f64() * 1000.0;
        let collect_refined_start = Instant::now();
        record_crash_phase_for_core(self, "add road terrain patch state");
        let invalidated_refined_cache_entries =
            self.refresh_road_locked_terrain_patch_state(ROAD_LOCKED_TERRAIN_RENDER_STEP_M);
        let collect_refined_ms = collect_refined_start.elapsed().as_secs_f64() * 1000.0;
        // The background jobs start last (`ROAD-68-G` part 7): neither reads the terrain patch
        // state, and started before it they took its workers' cores. The routing rebuild starts
        // when the lot plan ends (part 8), so the plan, which the next frame adopts, has the
        // background pool to itself; the rebuild's adoption frame comes later.
        if let Some(graph) = routing_graph {
            record_crash_phase_for_core(self, "add road cch rebuild scheduled");
            let routing_start = Instant::now();
            let routing = self
                .transit_network
                .schedule_cch_rebuild_unstarted(std::sync::Arc::clone(&graph));
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
        edit_metrics.generation = self.road_tool_surface_generation;
        edit_metrics.committed = road_add.committed;
        edit_metrics.core_work_ms =
            road_total.elapsed().as_secs_f64() * 1000.0 - edit_metrics.lock_wait_ms;
        edit_metrics.add_ms = add_internal_ms;
        edit_metrics.finalize_ms = finalize_ms;
        edit_metrics.surface_ms = surface_ms;
        edit_metrics.mesh_ms = mesh_ms;
        edit_metrics.snapshot_ms = snapshot_ms;
        self.last_road_edit_metrics = edit_metrics;
        let rejection_reason = if road_add.committed {
            String::new()
        } else {
            let rejection = self
                .last_road_rejection
                .take()
                .unwrap_or_else(|| RoadRejection::new("unknown", self.last_road_timing.clone()));
            rejection.report(
                RejectedBy::Commit,
                RoadAttempt {
                    points: &submitted_points,
                    fwd_lanes,
                    bkw_lanes,
                    snap: snap_to_existing_roads,
                },
                self.road_tool_surface_generation,
            );
            rejection.reason
        };
        RoadCommitOutcome {
            committed: road_add.committed,
            rejection_reason,
            rejection: if road_add.committed {
                String::new()
            } else {
                self.last_road_timing.clone()
            },
            road_snapshot,
            add_internal_ms,
            finalize_ms,
            surface_ms,
            mesh_ms,
            snapshot_ms,
            generation: self.road_tool_surface_generation,
        }
    }
}

impl SimCore {
    // `rebuild_building_entrances_internal` for a commit that queued no lot region
    // (`ROAD-68-G`): the same lot refresh and attachment repair, but only the entrances whose
    // inputs the edit may have changed are derived again (`repair_commit_entrances`).
    fn rebuild_building_entrances_for_commit(&mut self, dirty: &HashSet<usize>) {
        crate::thread_pools::install(|| {
            let cache_current = self.entrance_cache_current();
            let lots = self.prepare_cell_lots_internal();
            self.repair_commit_entrances(dirty, cache_current, &lots);
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
