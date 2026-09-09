// SPDX-License-Identifier: GPL-2.0-only

//! Background simulation command processing and fixed-rate thread loop.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use super::road_edit_plan::RoadEditPlan;
use super::road_preview::{
    RoadPreviewWorkerContext, RoadToolQuerySnapshot, road_tool_snapshots_from_core,
};
use super::snapshot::RenderSnapshot;
use super::state::SimCore;
use super::terrain_payloads::ROAD_LOCKED_TERRAIN_RENDER_STEP_M;
use crate::debug::{CrashCommand, CrashSimSnapshot};
use crate::debug_log;
use crate::nodes::sim::editing::BulldozeTarget;
use crate::simulation::network::road_edit::FinalizedRoadGeometry;
use godot::prelude::godot_error;

fn run_sim_phase<T>(phase: &str, run: impl FnOnce() -> T) -> T {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)) {
        Ok(value) => value,
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("(non-string payload)");
            godot_error!("[sim] {} panicked: {}", phase, message);
            crate::debug::flush_crash_diagnostics(phase);
            std::panic::resume_unwind(payload);
        }
    }
}

/// Commands sent from the Godot main thread to the sim background thread.
pub(crate) enum SimCommand {
    /// Update the simulation speed multiplier.
    SetSpeed(f32),
    /// Update the camera world-space AABB used for agent frustum culling.
    /// Values: (x_min, x_max, z_min, z_max) in world units, padded by ~200 m.
    SetCameraAabb(f32, f32, f32, f32),
    /// Place a new road segment.  Executed in the sim thread so the main thread
    /// never blocks on the expensive lane-rebuild and zoning-obstruction passes.
    AddRoad {
        /// World-space polyline points.
        points: Vec<godot::prelude::Vector3>,
        /// Forward lane count.
        fwd_lanes: i32,
        /// Backward lane count.
        bkw_lanes: i32,
        /// Whether authored endpoints may snap to nearby existing road nodes.
        snap_to_existing_roads: bool,
        /// Successful exact preview tied to the same immutable road-surface generation.
        edit_plan: Option<Arc<RoadEditPlan>>,
        /// Dispatch timestamp for queue latency, independent of core mutex contention.
        enqueued_at: Instant,
    },
    /// Undo the latest authoring operation entirely on the simulation thread.
    Undo,
    /// Delete one previously resolved building or road target on the simulation thread.
    Bulldoze {
        /// Immutable target token captured while the cursor still referenced the object.
        target: BulldozeTarget,
    },
}

fn finalize_network_render_products(
    core: &mut SimCore,
) -> Option<(RoadPreviewWorkerContext, RoadToolQuerySnapshot)> {
    core.rebuild_network_surface_terrain_internal();
    core.precompute_road_mesh_data();
    core.refresh_road_locked_terrain_patch_state(ROAD_LOCKED_TERRAIN_RENDER_STEP_M);
    road_tool_snapshots_from_core(core)
}

fn publish_road_tool_snapshots(
    road_preview_context: &RwLock<RoadPreviewWorkerContext>,
    road_query_snapshot: &RwLock<RoadToolQuerySnapshot>,
    preview_context: RoadPreviewWorkerContext,
    query_snapshot: RoadToolQuerySnapshot,
) {
    *road_preview_context
        .write()
        .expect("road preview context lock poisoned") = preview_context;
    *road_query_snapshot
        .write()
        .expect("road query snapshot lock poisoned") = query_snapshot;
}

fn crash_summary_from_core(core: &SimCore) -> CrashSimSnapshot {
    CrashSimSnapshot {
        day_index: core.time.day_index,
        minute_of_day: core.time.minute_of_day,
        speed_multiplier: core.time.speed_multiplier,
        agent_count: core.agents.len(),
        pathfind_count: core.agents.pathfind_count.load(Ordering::Relaxed),
        building_count: core.allocator.buildings.len(),
        household_count: core.households.households.len(),
        road_node_count: core.region_graph.node_count(),
        road_edge_count: core.region_graph.edge_count(),
        road_generation: core.road_tool_surface_generation,
        pending_demand_spawns: core.pending_demand_spawns.len(),
        last_agent_tick_us: core.last_agent_tick_us,
        last_tick_duration_ms: core.last_tick_duration,
        terrain_dirty: core.terrain_dirty,
        water_dirty: core.water_dirty,
        network_dirty: core.network_dirty,
    }
}

fn record_crash_phase_for_core(core: &SimCore, phase: &'static str) {
    if crate::debug::is_crash_diagnostics_enabled() {
        crate::debug::record_crash_phase(phase, crash_summary_from_core(core));
    }
}

fn record_crash_command_for_core(core: &SimCore, command: CrashCommand) {
    if crate::debug::is_crash_diagnostics_enabled() {
        crate::debug::record_crash_command(command, crash_summary_from_core(core));
    }
}

/// Background simulation thread loop.
///
/// Movement runs at ~60 Hz; queued edits wake the thread independently of that cadence.
/// Movement ticks and structural edits own the core mutex while they execute. Render-facing APIs consume
/// immutable snapshots or use nonblocking acquisition, and snapshot publication occurs
/// after releasing the core mutex.
pub(crate) fn run_sim_thread(
    core: Arc<Mutex<SimCore>>,
    snapshot: Arc<RwLock<RenderSnapshot>>,
    road_preview_context: Arc<RwLock<RoadPreviewWorkerContext>>,
    road_query_snapshot: Arc<RwLock<RoadToolQuerySnapshot>>,
    cmd_rx: std::sync::mpsc::Receiver<SimCommand>,
) {
    const TARGET_DT: f64 = 1.0 / 60.0;
    let target = Duration::from_micros(16_667); // ~60 Hz
    let mut recycled_snapshot = RenderSnapshot::default();
    let mut next_tick = Instant::now();
    let mut ready_command = None;
    // Coalesced controls survive wakes until the next tick or structural-edit snapshot.
    let mut pending_speed = None;
    let mut pending_camera_aabb = None;

    loop {
        let frame_start = Instant::now();

        // Drain all pending commands — non-blocking.
        let command_start = Instant::now();
        let mut commands_processed = 0_usize;
        let mut set_speed_commands = 0_usize;
        let mut camera_aabb_commands = 0_usize;
        let mut add_road_commands = 0_usize;
        let mut undo_commands = 0_usize;
        let mut bulldoze_commands = 0_usize;
        let mut should_quit = false;
        loop {
            // The command that interrupted the wait must precede newer queued commands.
            match ready_command
                .take()
                .map(Ok)
                .unwrap_or_else(|| cmd_rx.try_recv())
            {
                Ok(SimCommand::SetSpeed(s)) => {
                    commands_processed += 1;
                    set_speed_commands += 1;
                    pending_speed = Some(s);
                }
                Ok(SimCommand::SetCameraAabb(x0, x1, z0, z1)) => {
                    commands_processed += 1;
                    camera_aabb_commands += 1;
                    pending_camera_aabb = Some((x0, x1, z0, z1));
                }
                Ok(SimCommand::Undo) => {
                    commands_processed += 1;
                    undo_commands += 1;
                    let road_snapshots = {
                        let mut core = core.lock().expect("simulation core lock poisoned");
                        record_crash_command_for_core(&core, CrashCommand::Undo);
                        record_crash_phase_for_core(&core, "undo command");
                        let generation_before = core.road_tool_surface_generation;
                        if !core.undo_action_internal() {
                            None
                        } else if core.road_tool_surface_generation != generation_before {
                            record_crash_phase_for_core(&core, "undo network finalize");
                            finalize_network_render_products(&mut core)
                        } else {
                            None
                        }
                    };
                    if let Some((preview_context, query_snapshot)) = road_snapshots {
                        publish_road_tool_snapshots(
                            &road_preview_context,
                            &road_query_snapshot,
                            preview_context,
                            query_snapshot,
                        );
                    }
                }
                Ok(SimCommand::Bulldoze { target }) => {
                    commands_processed += 1;
                    bulldoze_commands += 1;
                    let road_snapshots = {
                        let mut core = core.lock().expect("simulation core lock poisoned");
                        record_crash_command_for_core(&core, CrashCommand::Bulldoze);
                        record_crash_phase_for_core(&core, "bulldoze command");
                        let road_deleted = core
                            .bulldoze_prepared_target_internal(target)
                            .unwrap_or(false);
                        if road_deleted {
                            record_crash_phase_for_core(&core, "bulldoze network finalize");
                            finalize_network_render_products(&mut core)
                        } else {
                            None
                        }
                    };
                    if let Some((preview_context, query_snapshot)) = road_snapshots {
                        publish_road_tool_snapshots(
                            &road_preview_context,
                            &road_query_snapshot,
                            preview_context,
                            query_snapshot,
                        );
                    }
                }
                Ok(SimCommand::AddRoad {
                    points,
                    fwd_lanes,
                    bkw_lanes,
                    snap_to_existing_roads,
                    edit_plan,
                    enqueued_at,
                }) => {
                    commands_processed += 1;
                    add_road_commands += 1;
                    let road_total = Instant::now();
                    let mut edit_metrics =
                        crate::nodes::sim::benchmark::road_edit::RoadEditMetrics::default();
                    edit_metrics.queue_wait_ms = enqueued_at.elapsed().as_secs_f64() * 1000.0;
                    let lock_wait_start = Instant::now();
                    let (
                        road_snapshots,
                        road_lock_wait_ms,
                        add_internal_ms,
                        finalize_ms,
                        surface_ms,
                        mesh_ms,
                        snapshot_ms,
                        collect_refined_ms,
                        invalidated_refined_cache_entries,
                    ) = {
                        let mut c = core.lock().expect("simulation core lock poisoned");
                        let road_lock_wait_ms = lock_wait_start.elapsed().as_secs_f64() * 1000.0;
                        record_crash_command_for_core(
                            &c,
                            CrashCommand::AddRoad {
                                point_count: points.len(),
                                fwd_lanes,
                                bkw_lanes,
                                snap_to_existing_roads,
                            },
                        );
                        // Bulk-load defers per-edge rebuilds until finalization.
                        let add_internal_start = Instant::now();
                        // Complete all geometry before opening the transaction. The worker and
                        // click rebuild use the same local compiler; neither clones the city here.
                        let edit_plan = edit_plan
                            .filter(|plan| {
                                plan.prepared_input_for(
                                    c.road_tool_surface_generation,
                                    c.heightmap.source_generation(),
                                    &points,
                                    fwd_lanes,
                                    bkw_lanes,
                                    snap_to_existing_roads,
                                )
                                .is_some()
                                    && plan.status(&c) == "ready"
                            })
                            .unwrap_or_else(|| {
                                Arc::new(super::RoadEditPlan::compile(
                                    &c,
                                    super::road_preview::RoadPreviewRequest {
                                        request_id: 0,
                                        surface_generation: c.road_tool_surface_generation,
                                        points: points.clone(),
                                        fwd_lanes,
                                        bkw_lanes,
                                        snap_to_existing_roads,
                                    },
                                ))
                            });
                        c.transit_network.bulk_load = true;
                        c.transit_network.begin_road_edit();
                        record_crash_phase_for_core(&c, "add road internal");
                        let mut road_add = c.add_road_internal_with_snap_and_validation(
                            points,
                            fwd_lanes,
                            bkw_lanes,
                            snap_to_existing_roads,
                            Some(edit_plan.as_ref()),
                        );
                        let add_internal_ms = add_internal_start.elapsed().as_secs_f64() * 1000.0;
                        let finalize_start = Instant::now();
                        let mut surface_ms = 0.0;
                        if road_add.committed {
                            let c = &mut *c;
                            c.transit_network.bulk_load = false;
                            record_crash_phase_for_core(c, "add road geometry finalize");

                            let terrain_plan = road_add
                                .finalized_geometry
                                .as_ref()
                                .map(|_| edit_plan.as_ref())
                                .and_then(|plan| plan.terrain());

                            let FinalizedRoadGeometry {
                                dirty_edges: dirty,
                                affected_nodes: _affected_nodes,
                                profile_us: dt_profile_us,
                                clips_us: dt_clips_us,
                            } = road_add
                                .finalized_geometry
                                .take()
                                .unwrap_or_else(|| c.finalize_bulk_road_geometry_for_dirty_edges());
                            let dirty_count = dirty.len();
                            edit_metrics.dirty_edges = dirty_count;
                            if crate::debug::category_enabled("road")
                                && std::env::var("METRUM_DEBUG_ROAD_GEOMETRY_DUMP")
                                    .map(|value| !value.is_empty() && value != "0")
                                    .unwrap_or(false)
                            {
                                c.last_surface_debug_edges.extend(dirty.iter().copied());
                                c.last_surface_debug_edges.sort_unstable();
                                c.last_surface_debug_edges.dedup();
                            }

                            if let Some(topology_reuse) = road_add.preview_topology_reuse.take() {
                                c.transit_network
                                    .road_surface
                                    .enqueue_preview_topology_reuse(topology_reuse);
                            }
                            let surface_start = Instant::now();
                            record_crash_phase_for_core(c, "add road render validation");
                            road_add.committed =
                                c.validate_staged_road_render_with_plan(terrain_plan);
                            surface_ms = surface_start.elapsed().as_secs_f64() * 1000.0;
                            if road_add.committed {
                                let t_inv = Instant::now();
                                // Invalidate agents BEFORE lane rebuild so old lane IDs are still valid.
                                record_crash_phase_for_core(c, "add road lane invalidation");
                                c.agents.invalidate_lane_ids_for_edges(
                                    &dirty,
                                    &c.transit_network.lane_system,
                                    &c.region_graph,
                                );
                                let dt_inv_us = t_inv.elapsed().as_micros();
                                edit_metrics.agent_invalidate_ms = dt_inv_us as f64 / 1000.0;

                                let t_lanes = Instant::now();
                                record_crash_phase_for_core(c, "add road lane rebuild");
                                c.transit_network
                                    .lane_system
                                    .rebuild_edges_incremental(&mut c.region_graph, &dirty);
                                c.agents.reattach_invalidated_lanes_for_edges(
                                    &dirty,
                                    &c.transit_network.lane_system,
                                    &c.region_graph,
                                );
                                let dt_lanes_us = t_lanes.elapsed().as_micros();
                                edit_metrics.lanes_and_reattach_ms = dt_lanes_us as f64 / 1000.0;
                                let buildings_start = Instant::now();
                                record_crash_phase_for_core(c, "add road entrance rebuild");
                                c.rebuild_building_entrances_internal();
                                edit_metrics.buildings_ms =
                                    buildings_start.elapsed().as_secs_f64() * 1000.0;

                                // Rebuild CCH and run the connectivity check. This is the only
                                // place the CCH is actually rebuilt for road placements — the
                                // sim-tick path is gated on speed > 0.0 and would miss paused edits.
                                record_crash_phase_for_core(c, "add road cch rebuild");
                                let routing_start = Instant::now();
                                c.transit_network.rebuild_cch_and_check(&c.region_graph);
                                c.transit_network.cch_dirty_chunks.clear();
                                edit_metrics.routing_ms =
                                    routing_start.elapsed().as_secs_f64() * 1000.0;

                                // Zone flush is deferred to the next simulate_tick_internal call
                                // so it does not block road placement. zoning_dirty_edges accumulates.

                                let total_us = road_total.elapsed().as_micros();
                                let msg = format!(
                                    "TOTAL={}µs  {}  profiles={}µs  clips={}µs  lanes={}µs({}e)  invalidate={}µs",
                                    total_us,
                                    c.last_road_timing,
                                    dt_profile_us,
                                    dt_clips_us,
                                    dt_lanes_us,
                                    dirty_count,
                                    dt_inv_us
                                );
                                debug_log!("road", "{}", msg);
                                c.last_road_timing = msg;
                                if !c.benchmark_mode {
                                    c.treasury.deduct_build_cost(road_add.build_cost);
                                }
                            }
                        } else {
                            c.transit_network.bulk_load = false;
                            c.transit_network.accept_road_edit();
                        }
                        let finalize_ms =
                            (finalize_start.elapsed().as_secs_f64() * 1000.0 - surface_ms).max(0.0);
                        let mesh_start = Instant::now();
                        record_crash_phase_for_core(&c, "add road mesh precompute");
                        c.precompute_road_mesh_data();
                        let mesh_ms = mesh_start.elapsed().as_secs_f64() * 1000.0;
                        let snapshot_start = Instant::now();
                        record_crash_phase_for_core(&c, "add road tool snapshot");
                        let road_snapshots = road_tool_snapshots_from_core(&c);
                        let snapshot_ms = snapshot_start.elapsed().as_secs_f64() * 1000.0;
                        let collect_refined_start = Instant::now();
                        record_crash_phase_for_core(&c, "add road terrain patch state");
                        let invalidated_refined_cache_entries = c
                            .refresh_road_locked_terrain_patch_state(
                                ROAD_LOCKED_TERRAIN_RENDER_STEP_M,
                            );
                        let collect_refined_ms =
                            collect_refined_start.elapsed().as_secs_f64() * 1000.0;
                        edit_metrics.generation = c.road_tool_surface_generation;
                        edit_metrics.committed = road_add.committed;
                        edit_metrics.lock_wait_ms = road_lock_wait_ms;
                        edit_metrics.core_work_ms =
                            road_total.elapsed().as_secs_f64() * 1000.0 - road_lock_wait_ms;
                        edit_metrics.add_ms = add_internal_ms;
                        edit_metrics.finalize_ms = finalize_ms;
                        edit_metrics.surface_ms = surface_ms;
                        edit_metrics.mesh_ms = mesh_ms;
                        edit_metrics.snapshot_ms = snapshot_ms;
                        edit_metrics.refined_state_ms = collect_refined_ms;
                        edit_metrics.rebuilt_surface_chunks = c
                            .transit_network
                            .road_surface
                            .last_rebuilt_surface_chunks
                            .len();
                        edit_metrics.rebuilt_terrain_chunks = c
                            .transit_network
                            .road_surface
                            .last_rebuilt_terrain_chunks
                            .len();
                        c.last_road_edit_metrics = edit_metrics;
                        (
                            road_snapshots,
                            road_lock_wait_ms,
                            add_internal_ms,
                            finalize_ms,
                            surface_ms,
                            mesh_ms,
                            snapshot_ms,
                            collect_refined_ms,
                            invalidated_refined_cache_entries,
                        )
                    };
                    if let Some((preview_context, query_snapshot)) = road_snapshots {
                        publish_road_tool_snapshots(
                            &road_preview_context,
                            &road_query_snapshot,
                            preview_context,
                            query_snapshot,
                        );
                    }
                    if crate::debug::is_perf_enabled() {
                        println!(
                            "[DEBUG:perf] add_road_command total_ms={:.3} lock_wait_ms={:.3} add_internal_ms={:.3} finalize_ms={:.3} surface_and_terrain_ms={:.3} mesh_ms={:.3} snapshot_ms={:.3} collect_refined_ms={:.3} refined_cache_invalidated={} refined_prebuild=validated_before_accept",
                            road_total.elapsed().as_secs_f64() * 1000.0,
                            road_lock_wait_ms,
                            add_internal_ms,
                            finalize_ms,
                            surface_ms,
                            mesh_ms,
                            snapshot_ms,
                            collect_refined_ms,
                            invalidated_refined_cache_entries
                        );
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    should_quit = true;
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
            }
        }
        let command_ms = command_start.elapsed().as_secs_f64() * 1000.0;
        if should_quit {
            crate::debug::suspend_hang_watchdog();
            return;
        }

        let now = Instant::now();
        let tick_due = begin_due_sim_tick(&mut next_tick, now, target);
        let edit_snapshot_due = add_road_commands + undo_commands + bulldoze_commands > 0;
        if !tick_due && !edit_snapshot_due {
            // O(1), allocation-free scheduling on the existing channel. Authoring commands wake
            // immediately, but neither wakeups nor expensive edits create extra movement ticks.
            ready_command = match cmd_rx.recv_timeout(next_tick.saturating_duration_since(now)) {
                Ok(command) => Some(command),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    crate::debug::suspend_hang_watchdog();
                    return;
                }
            };
            continue;
        }

        let perf_enabled = crate::debug::is_perf_enabled();
        let lock_wait_ms: f64;
        let mut pathing_ms = 0.0;
        let mut agent_ms = 0.0;
        let mut minute_ms = 0.0;
        let mut pending_spawn_ms = 0.0;
        let mut hourly_ms = 0.0;
        let mut daily_ms = 0.0;
        let snapshot_ms: f64;
        let lock_held_ms: f64;
        let mut elapsed_minutes = 0_u16;
        let mut pending_spawns_executed = 0_usize;
        let mut hourly_ticks = 0_usize;
        let mut daily_ticks = 0_usize;
        let agent_count: i32;
        let pathfind_count: u32;
        let crash_frame_summary: Option<CrashSimSnapshot>;

        // Tick and build snapshot inside one lock acquisition.
        let new_snapshot = {
            let lock_wait_start = Instant::now();
            let mut core = core.lock().expect("simulation core lock poisoned");
            lock_wait_ms = lock_wait_start.elapsed().as_secs_f64() * 1000.0;
            let lock_held_start = Instant::now();
            if let Some(speed) = pending_speed.take() {
                core.time.speed_multiplier = speed;
                record_crash_command_for_core(&core, CrashCommand::SetSpeed { speed });
            }
            if let Some(camera_aabb) = pending_camera_aabb.take() {
                core.camera_aabb = camera_aabb;
                record_crash_command_for_core(
                    &core,
                    CrashCommand::SetCameraAabb {
                        x_min: camera_aabb.0,
                        x_max: camera_aabb.1,
                        z_min: camera_aabb.2,
                        z_max: camera_aabb.3,
                    },
                );
            }
            record_crash_phase_for_core(&core, "sim frame");
            let speed = core.time.speed_multiplier;

            // Publish completed edits immediately, even between movement deadlines. Otherwise
            // network/terrain dirty flags would put the removed queue delay back on the renderer.
            if tick_due && speed > 0.0 {
                // Rebuild CCH if dirty, then rebuild any dirty flow fields.
                let pathing_start = Instant::now();
                let c = &mut *core;
                record_crash_phase_for_core(c, "pathing rebuild");
                c.transit_network
                    .rebuild_pathing_if_dirty(&mut c.region_graph);
                {
                    let alloc = &c.allocator;
                    let graph = &c.region_graph;
                    c.transit_network
                        .flow_fields
                        .rebuild_dirty(graph, |zone, mode_flags| {
                            alloc.get_sources_for_zone(zone, graph, mode_flags)
                        });
                }
                pathing_ms = pathing_start.elapsed().as_secs_f64() * 1000.0;

                let dt = (TARGET_DT * speed as f64) as f32;
                let t_agent = Instant::now();

                record_crash_phase_for_core(&core, "agent tick");
                run_sim_phase("agent tick", || {
                    let c = &mut *core;
                    c.agents.tick(
                        &c.allocator,
                        &mut c.transit_network,
                        &mut c.region_graph,
                        dt,
                        c.time.day_index,
                        c.time.minute_of_day,
                    );
                });

                core.last_agent_tick_us = t_agent.elapsed().as_micros() as u64;
                agent_ms = core.last_agent_tick_us as f64 / 1000.0;

                let minute_start = Instant::now();
                record_crash_phase_for_core(&core, "time advance");
                let time_advance = core.time.process_delta(TARGET_DT);
                elapsed_minutes = time_advance.elapsed_minutes;
                if time_advance.has_elapsed_minutes() {
                    for (step_day_index, step_minute_of_day) in time_advance.iter_elapsed_minutes()
                    {
                        let pending_spawn_start = Instant::now();
                        record_crash_phase_for_core(&core, "demand spawn tick");
                        pending_spawns_executed += run_sim_phase("demand spawn tick", || {
                            core.execute_pending_demand_spawns_for_minute(
                                step_day_index,
                                step_minute_of_day,
                            )
                        });
                        pending_spawn_ms += pending_spawn_start.elapsed().as_secs_f64() * 1000.0;
                        if step_minute_of_day % 60 == 0 {
                            let hourly_start = Instant::now();
                            record_crash_phase_for_core(&core, "operational hour tick");
                            run_sim_phase("operational hour tick", || {
                                core.simulate_operational_hour_internal(
                                    step_day_index,
                                    step_minute_of_day,
                                )
                            });
                            hourly_ms += hourly_start.elapsed().as_secs_f64() * 1000.0;
                            hourly_ticks += 1;
                            if step_minute_of_day != 0 && crate::debug::is_sim_enabled() {
                                core.print_sim_console_summary(step_day_index, step_minute_of_day);
                            }
                        }
                        if step_minute_of_day == 0 {
                            let daily_start = Instant::now();
                            record_crash_phase_for_core(&core, "daily tick");
                            run_sim_phase("daily tick", || {
                                core.simulate_tick_internal(step_day_index)
                            });
                            daily_ms += daily_start.elapsed().as_secs_f64() * 1000.0;
                            daily_ticks += 1;
                            if crate::debug::is_sim_enabled() {
                                core.print_sim_console_summary(step_day_index, step_minute_of_day);
                            }
                            core.print_daily_building_economy_for_day(step_day_index);
                        }
                    }
                }
                minute_ms = minute_start.elapsed().as_secs_f64() * 1000.0;
            }

            let snapshot_start = Instant::now();
            let available_snapshot = std::mem::take(&mut recycled_snapshot);
            record_crash_phase_for_core(&core, "snapshot build");
            let snapshot = run_sim_phase("snapshot build", || {
                core.build_snapshot_reusing(available_snapshot)
            });
            snapshot_ms = snapshot_start.elapsed().as_secs_f64() * 1000.0;
            lock_held_ms = lock_held_start.elapsed().as_secs_f64() * 1000.0;
            agent_count = snapshot.agent_count;
            pathfind_count = snapshot.pathfind_count;
            crash_frame_summary = crate::debug::is_crash_diagnostics_enabled()
                .then(|| crash_summary_from_core(&core));
            snapshot
        };

        // Write snapshot — outside the sim lock so render reads are non-blocking.
        let snapshot_write_start = Instant::now();
        let previous_snapshot = {
            let mut current = snapshot.write().expect("render snapshot lock poisoned");
            std::mem::replace(&mut *current, new_snapshot)
        };
        recycled_snapshot = previous_snapshot;
        let snapshot_write_ms = snapshot_write_start.elapsed().as_secs_f64() * 1000.0;
        let active_ms = frame_start.elapsed().as_secs_f64() * 1000.0;
        if let Some(summary) = crash_frame_summary {
            crate::debug::record_crash_frame(
                summary,
                active_ms,
                command_ms,
                lock_wait_ms,
                lock_held_ms,
                snapshot_ms,
                snapshot_write_ms,
                elapsed_minutes,
                pending_spawns_executed,
                hourly_ticks,
                daily_ticks,
                commands_processed,
            );
        }
        let unaccounted_ms =
            (active_ms - command_ms - lock_wait_ms - lock_held_ms - snapshot_write_ms).max(0.0);
        if perf_enabled && (active_ms >= 8.0 || command_ms >= 8.0 || elapsed_minutes > 0) {
            println!(
                "[DEBUG:perf] sim_frame active_ms={:.3} command_ms={:.3} lock_wait_ms={:.3} lock_held_ms={:.3} pathing_ms={:.3} agent_ms={:.3} minute_ms={:.3} pending_spawn_ms={:.3} hourly_ms={:.3} daily_ms={:.3} snapshot_ms={:.3} snapshot_write_ms={:.3} unaccounted_ms={:.3} elapsed_minutes={} pending_spawns={} hourly_ticks={} daily_ticks={} agents={} pathfinds={} commands={} set_speed_cmds={} camera_aabb_cmds={} add_road_cmds={} undo_cmds={} bulldoze_cmds={}",
                active_ms,
                command_ms,
                lock_wait_ms,
                lock_held_ms,
                pathing_ms,
                agent_ms,
                minute_ms,
                pending_spawn_ms,
                hourly_ms,
                daily_ms,
                snapshot_ms,
                snapshot_write_ms,
                unaccounted_ms,
                elapsed_minutes,
                pending_spawns_executed,
                hourly_ticks,
                daily_ticks,
                agent_count,
                pathfind_count,
                commands_processed,
                set_speed_commands,
                camera_aabb_commands,
                add_road_commands,
                undo_commands,
                bulldoze_commands,
            );
        }
    }
}

// Missed wall-clock deadlines never trigger catch-up bursts. Fixed simulation dt remains owned
// by the tick body; this gate only decides when that body may run again.
fn begin_due_sim_tick(next_tick: &mut Instant, now: Instant, interval: Duration) -> bool {
    if now < *next_tick {
        return false;
    }
    *next_tick = now + interval;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_wakes_neither_advance_nor_postpone_sim_ticks() {
        let start = Instant::now();
        let interval = Duration::from_micros(16_667);
        let mut next_tick = start;
        for tick in 0..60 {
            for offset_us in [0, 1, 100, 8_000, 16_666] {
                let now = start + interval * tick + Duration::from_micros(offset_us);
                assert_eq!(
                    begin_due_sim_tick(&mut next_tick, now, interval),
                    offset_us == 0
                );
            }
        }
        assert_eq!(next_tick, start + interval * 60);
    }

    #[test]
    fn slow_road_command_does_not_create_catch_up_ticks() {
        let start = Instant::now();
        let interval = Duration::from_micros(16_667);
        let mut next_tick = start;
        assert!(begin_due_sim_tick(&mut next_tick, start, interval));
        let late = start + interval * 5;
        assert!(begin_due_sim_tick(&mut next_tick, late, interval));
        assert!(!begin_due_sim_tick(&mut next_tick, late, interval));
        assert_eq!(next_tick, late + interval);
    }
}
