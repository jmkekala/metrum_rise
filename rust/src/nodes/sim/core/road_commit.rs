// SPDX-License-Identifier: GPL-2.0-only

//! Authoritative road placement: plan reuse, transactional commit and dependent rebuilds.
//!
//! The sim thread runs this for every `SimCommand::AddRoad`; headless scenarios call it to lay
//! roads through the same path as the player.

use std::sync::Arc;
use std::time::Instant;

use super::frame::record_crash_phase_for_core;
use super::road_edit_plan::RoadEditPlan;
use super::road_preview::{
    RoadPreviewRequest, RoadPreviewWorkerContext, RoadToolQuerySnapshot,
    road_tool_snapshots_from_core,
};
use super::state::SimCore;
use super::terrain_payloads::ROAD_LOCKED_TERRAIN_RENDER_STEP_M;
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
    /// Successful exact preview; reused only while its inputs still match the core.
    pub(crate) edit_plan: Option<Arc<RoadEditPlan>>,
}

/// Result and phase timings of [`SimCore::commit_road`].
pub(crate) struct RoadCommitOutcome {
    /// Whether the road was accepted.
    pub(crate) committed: bool,
    /// Rejection diagnostic; empty when committed.
    pub(crate) rejection: String,
    /// Road-tool snapshots to publish after releasing the core lock.
    pub(crate) road_snapshots: Option<(RoadPreviewWorkerContext, RoadToolQuerySnapshot)>,
    pub(crate) add_internal_ms: f64,
    pub(crate) finalize_ms: f64,
    pub(crate) surface_ms: f64,
    pub(crate) mesh_ms: f64,
    pub(crate) snapshot_ms: f64,
    pub(crate) collect_refined_ms: f64,
    pub(crate) invalidated_refined_cache_entries: usize,
}

impl SimCore {
    /// Validates and commits one road stroke, then refreshes lanes, agents, entrances, routing,
    /// road meshes and road-locked terrain. A rejected stroke leaves the network unchanged.
    ///
    /// `road_total` starts the command's wall-clock measurement; `edit_metrics` arrives with
    /// queue and lock waits filled in and is stored as `last_road_edit_metrics`.
    pub(crate) fn commit_road(
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
            edit_plan,
        } = request;
        // Bulk-load defers per-edge rebuilds until finalization.
        let add_internal_start = Instant::now();
        // Complete all geometry before opening the transaction. The worker and
        // click rebuild use the same local compiler; neither clones the city here.
        let edit_plan = edit_plan.filter(|plan| {
            plan.prepared_input_for(
                self.road_tool_surface_generation,
                self.heightmap.source_generation(),
                &points,
                fwd_lanes,
                bkw_lanes,
                snap_to_existing_roads,
            )
            .is_some()
                && plan.road_status(self) == "ready"
        });
        edit_metrics.preview_plan_reused = edit_plan.is_some();
        let road_plan = edit_plan.unwrap_or_else(|| {
            Arc::new(RoadEditPlan::compile_road(
                self,
                RoadPreviewRequest {
                    enqueued_at: None,
                    include_terrain: false,
                    request_id: 0,
                    surface_generation: self.road_tool_surface_generation,
                    points: points.clone(),
                    fwd_lanes,
                    bkw_lanes,
                    snap_to_existing_roads,
                },
            ))
        });
        edit_metrics.road_plan_ms = add_internal_start.elapsed().as_secs_f64() * 1000.0;
        let terrain_start = Instant::now();
        let edit_plan = road_plan.complete_for_commit(self);
        edit_metrics.terrain_plan_ms = terrain_start.elapsed().as_secs_f64() * 1000.0;
        self.transit_network.bulk_load = true;
        self.transit_network.begin_road_edit();
        record_crash_phase_for_core(self, "add road internal");
        let mut road_add = self.add_road_internal_with_snap_and_validation(
            points,
            fwd_lanes,
            bkw_lanes,
            snap_to_existing_roads,
            Some(&edit_plan),
        );
        let add_internal_ms = add_internal_start.elapsed().as_secs_f64() * 1000.0;
        let finalize_start = Instant::now();
        let mut surface_ms = 0.0;
        if road_add.committed {
            self.transit_network.bulk_load = false;
            record_crash_phase_for_core(self, "add road geometry finalize");

            let terrain_plan = road_add
                .finalized_geometry
                .as_ref()
                .map(|_| &edit_plan)
                .and_then(|plan| plan.terrain());

            let FinalizedRoadGeometry {
                dirty_edges: dirty,
                affected_nodes: _affected_nodes,
                profile_us: dt_profile_us,
                clips_us: dt_clips_us,
            } = road_add
                .finalized_geometry
                .take()
                .unwrap_or_else(|| self.finalize_bulk_road_geometry_for_dirty_edges());
            let dirty_count = dirty.len();
            edit_metrics.dirty_edges = dirty_count;
            if crate::debug::category_enabled("road")
                && std::env::var("METRUM_DEBUG_ROAD_GEOMETRY_DUMP")
                    .map(|value| !value.is_empty() && value != "0")
                    .unwrap_or(false)
            {
                self.last_surface_debug_edges.extend(dirty.iter().copied());
                self.last_surface_debug_edges.sort_unstable();
                self.last_surface_debug_edges.dedup();
            }

            if let Some(topology_reuse) = road_add.preview_topology_reuse.take() {
                self.transit_network
                    .road_surface
                    .enqueue_preview_topology_reuse(topology_reuse);
            }
            let surface_start = Instant::now();
            record_crash_phase_for_core(self, "add road render validation");
            road_add.committed = self.validate_staged_road_render_with_plan(terrain_plan);
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
                self.rebuild_building_entrances_internal();
                edit_metrics.buildings_ms = buildings_start.elapsed().as_secs_f64() * 1000.0;

                // Rebuild CCH and run the connectivity check. This is the only
                // place the CCH is actually rebuilt for road placements — the
                // sim-tick path is gated on speed > 0.0 and would miss paused edits.
                record_crash_phase_for_core(self, "add road cch rebuild");
                let routing_start = Instant::now();
                self.transit_network
                    .rebuild_cch_and_check(&self.region_graph);
                self.transit_network.cch_dirty_chunks.clear();
                edit_metrics.routing_ms = routing_start.elapsed().as_secs_f64() * 1000.0;

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
        let road_snapshots = road_tool_snapshots_from_core(self);
        let snapshot_ms = snapshot_start.elapsed().as_secs_f64() * 1000.0;
        let collect_refined_start = Instant::now();
        record_crash_phase_for_core(self, "add road terrain patch state");
        let invalidated_refined_cache_entries =
            self.refresh_road_locked_terrain_patch_state(ROAD_LOCKED_TERRAIN_RENDER_STEP_M);
        let collect_refined_ms = collect_refined_start.elapsed().as_secs_f64() * 1000.0;
        edit_metrics.generation = self.road_tool_surface_generation;
        edit_metrics.committed = road_add.committed;
        edit_metrics.core_work_ms =
            road_total.elapsed().as_secs_f64() * 1000.0 - edit_metrics.lock_wait_ms;
        edit_metrics.add_ms = add_internal_ms;
        edit_metrics.finalize_ms = finalize_ms;
        edit_metrics.surface_ms = surface_ms;
        edit_metrics.mesh_ms = mesh_ms;
        edit_metrics.snapshot_ms = snapshot_ms;
        edit_metrics.refined_state_ms = collect_refined_ms;
        edit_metrics.rebuilt_surface_chunks = self
            .transit_network
            .road_surface
            .last_rebuilt_surface_chunks
            .len();
        edit_metrics.rebuilt_terrain_chunks = self
            .transit_network
            .road_surface
            .last_rebuilt_terrain_chunks
            .len();
        self.last_road_edit_metrics = edit_metrics;
        RoadCommitOutcome {
            committed: road_add.committed,
            rejection: if road_add.committed {
                String::new()
            } else {
                self.last_road_timing.clone()
            },
            road_snapshots,
            add_internal_ms,
            finalize_ms,
            surface_ms,
            mesh_ms,
            snapshot_ms,
            collect_refined_ms,
            invalidated_refined_cache_entries,
        }
    }
}
