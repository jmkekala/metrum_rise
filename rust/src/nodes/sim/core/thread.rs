// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: thread.rs
//  script_path: rust/src/nodes/sim/core/thread.rs
//  module_name: thread
//  version: 0.1.0
//  description: Background simulation command processing and fixed-rate
//           thread loop.
//  kind: module
//  spec: none
//  internal_dependencies: []
//  external_dependencies: []
//  features: []
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-27
// ========================================================================

//! Background simulation command processing and fixed-rate thread loop.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use super::road_preview::{
    RoadPreviewWorkerContext, RoadToolQuerySnapshot, road_tool_snapshots_from_core,
};
use super::snapshot::RenderSnapshot;
use super::state::{BulkRoadGeometryFinalize, SimCore};
use super::terrain_payloads::ROAD_LOCKED_TERRAIN_RENDER_STEP_M;
use crate::debug::{CrashCommand, CrashSimSnapshot};
use crate::debug_log;
use crate::nodes::sim::editing::BulldozeTarget;
use godot::prelude::godot_error;

// ========================================================================
// PHASE GUARDS
// ========================================================================

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
        /// An authored cross-section, in the flat form `LaneLayout::from_flat`
        /// reads. When present the counts above are ignored and derived from
        /// it instead, so a road with a median or a bus lane is placed as what
        /// it is rather than as the nearest pair of numbers.
        cross_section: Option<Vec<i32>>,
        /// Whether authored endpoints may snap to nearby existing road nodes.
        snap_to_existing_roads: bool,
    },
    /// Spawn looping car traffic across the existing road graph.
    ///
    /// Runs on the sim thread because it pathfinds and mutates the agent
    /// arrays, both of which need the core lock the sim thread already holds
    /// each tick. Calling the equivalent from the main thread deadlocks against
    /// it, which is why `setup_benchmark_city` may only be used before the
    /// thread spawns.
    SpawnTestTraffic {
        /// How many cars to put on the road.
        count: i32,
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

// ========================================================================
// THE THREAD LOOP
// ========================================================================

/// Background simulation thread loop.
///
/// Runs at ~60 Hz, decoupled from Godot's render frame. Movement ticks and queued
/// structural edits own the core mutex while they execute. Render-facing APIs consume
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
        let mut pending_speed = None;
        let mut pending_camera_aabb = None;
        let mut should_quit = false;
        loop {
            match cmd_rx.try_recv() {
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
                Ok(SimCommand::SpawnTestTraffic { count }) => {
                    commands_processed += 1;
                    let mut core = core.lock().expect("simulation core lock poisoned");
                    core.spawn_test_traffic_internal(count);
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
                    cross_section,
                    snap_to_existing_roads,
                }) => {
                    commands_processed += 1;
                    add_road_commands += 1;
                    let road_total = Instant::now();
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
                        c.transit_network.bulk_load = true;
                        record_crash_phase_for_core(&c, "add road internal");
                        let road_add = c.add_road_internal_with_cross_section(
                            points,
                            fwd_lanes,
                            bkw_lanes,
                            cross_section.as_deref(),
                            snap_to_existing_roads,
                        );
                        let add_internal_ms = add_internal_start.elapsed().as_secs_f64() * 1000.0;
                        let finalize_start = Instant::now();
                        if road_add.committed {
                            let c = &mut *c;
                            c.transit_network.bulk_load = false;
                            record_crash_phase_for_core(c, "add road geometry finalize");

                            let BulkRoadGeometryFinalize {
                                dirty_edges: dirty,
                                affected_nodes: _affected_nodes,
                                profile_us: dt_profile_us,
                                regrade_us: dt_regrade_us,
                                clips_us: dt_clips_us,
                            } = c.finalize_bulk_road_geometry_for_dirty_edges();
                            let dirty_count = dirty.len();
                            if crate::debug::category_enabled("road")
                                && std::env::var("METRUM_DEBUG_ROAD_GEOMETRY_DUMP")
                                    .map(|value| !value.is_empty() && value != "0")
                                    .unwrap_or(false)
                            {
                                c.last_surface_debug_edges.extend(dirty.iter().copied());
                                c.last_surface_debug_edges.sort_unstable();
                                c.last_surface_debug_edges.dedup();
                            }

                            let t_inv = Instant::now();
                            // Invalidate agents BEFORE lane rebuild so old lane IDs are still valid.
                            record_crash_phase_for_core(c, "add road lane invalidation");
                            c.agents.invalidate_lane_ids_for_edges(
                                &dirty,
                                &c.transit_network.lane_system,
                                &c.region_graph,
                            );
                            let dt_inv_us = t_inv.elapsed().as_micros();

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
                            record_crash_phase_for_core(c, "add road entrance rebuild");
                            c.rebuild_building_entrances_internal();

                            // Rebuild CCH and run the connectivity check. This is the only
                            // place the CCH is actually rebuilt for road placements — the
                            // sim-tick path is gated on speed > 0.0 and would miss paused edits.
                            record_crash_phase_for_core(c, "add road cch rebuild");
                            c.transit_network.rebuild_cch_and_check(&c.region_graph);
                            c.transit_network.cch_dirty_chunks.clear();

                            // Zone flush is deferred to the next simulate_tick_internal call
                            // so it does not block road placement. zoning_dirty_edges accumulates.

                            let total_us = road_total.elapsed().as_micros();
                            let msg = format!(
                                "TOTAL={}µs  {}  profiles={}µs  regrade={}µs  clips={}µs  lanes={}µs({}e)  invalidate={}µs",
                                total_us,
                                c.last_road_timing,
                                dt_profile_us,
                                dt_regrade_us,
                                dt_clips_us,
                                dt_lanes_us,
                                dirty_count,
                                dt_inv_us
                            );
                            debug_log!("road", "{}", msg);
                            c.last_road_timing = msg;
                        } else {
                            c.transit_network.bulk_load = false;
                        }
                        let finalize_ms = finalize_start.elapsed().as_secs_f64() * 1000.0;
                        let surface_start = Instant::now();
                        if road_add.committed {
                            record_crash_phase_for_core(&c, "add road surface rebuild");
                            c.rebuild_network_surface_terrain_internal_with_entrance_rebuild(false);
                            if !c
                                .transit_network
                                .road_surface
                                .published_generation_matches_source()
                            {
                                c.rollback_unpublishable_road_commit();
                            } else if !c.benchmark_mode {
                                c.treasury.deduct_build_cost(road_add.build_cost);
                            }
                        }
                        let surface_ms = surface_start.elapsed().as_secs_f64() * 1000.0;
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
                    let refined_input_count = 0usize;
                    let refined_window_count = 0usize;
                    let refined_reused_windows = 0usize;
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
                            "[DEBUG:perf] add_road_command total_ms={:.3} lock_wait_ms={:.3} add_internal_ms={:.3} finalize_ms={:.3} surface_ms={:.3} mesh_ms={:.3} snapshot_ms={:.3} collect_refined_ms={:.3} refined_build_ms={:.3} refined_cdt_sum_ms={:.3} refined_inputs={} refined_entries={} refined_windows={} refined_reused_windows={} refined_cache_invalidated={} insert_lock_wait_ms={:.3} insert_ms={:.3} refined_prebuild=skipped",
                            road_total.elapsed().as_secs_f64() * 1000.0,
                            road_lock_wait_ms,
                            add_internal_ms,
                            finalize_ms,
                            surface_ms,
                            mesh_ms,
                            snapshot_ms,
                            collect_refined_ms,
                            0.0,
                            0.0,
                            refined_input_count,
                            0,
                            refined_window_count,
                            refined_reused_windows,
                            invalidated_refined_cache_entries,
                            0.0,
                            0.0
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
            if let Some(speed) = pending_speed {
                core.time.speed_multiplier = speed;
                record_crash_command_for_core(&core, CrashCommand::SetSpeed { speed });
            }
            if let Some(camera_aabb) = pending_camera_aabb {
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

            if speed > 0.0 {
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

        // Sleep to maintain ~60 Hz.
        let elapsed = frame_start.elapsed();
        if elapsed < target {
            std::thread::sleep(target - elapsed);
        }
    }
}
