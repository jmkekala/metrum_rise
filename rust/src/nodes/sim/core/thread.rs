// SPDX-License-Identifier: GPL-2.0-only

//! Background simulation command processing and fixed-rate thread loop.

use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use super::frame::{
    FRAME_DT_S, FrameStepReport, crash_summary_from_core, record_crash_phase_for_core,
    run_sim_phase,
};
use super::road_commit::RoadCommitRequest;
use super::road_edit_plan::RoadEditPlan;
use super::road_preview::{
    RoadPreviewWorkerContext, RoadToolQuerySnapshot, road_tool_snapshots_from_core,
};
use super::snapshot::RenderSnapshot;
use super::state::SimCore;
use super::terrain_payloads::ROAD_LOCKED_TERRAIN_RENDER_STEP_M;
use crate::debug::{CrashCommand, CrashSimSnapshot};
use crate::nodes::sim::editing::BulldozeTarget;

/// Commands sent from the Godot main thread to the sim background thread.
pub(crate) enum SimCommand {
    /// Update the simulation speed multiplier.
    SetSpeed(f32),
    /// Update the camera world-space AABB used for agent frustum culling.
    /// Values: (x_min, x_max, z_min, z_max) in world units, padded by ~200 m.
    SetCameraAabb(f32, f32, f32, f32),
    /// Show or hide the zoning cell overlay; shown overlays regenerate edited chunks eagerly.
    SetCellOverlayVisible(bool),
    /// Prepare one visible cell chunk using current authority, including while paused.
    PrepareCellChunk {
        /// World-zero spatial chunk address.
        chunk: (i32, i32),
        /// Global terrain payload epoch, advanced by world replacement and loading.
        world_generation: u64,
        /// Releases the bridge's single outstanding request after completion or stale rejection.
        completion: std::sync::mpsc::SyncSender<()>,
    },
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
        /// Single nonblocking acknowledgment after acceptance/rollback and query publication.
        completion: std::sync::mpsc::SyncSender<(bool, String)>,
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
    let target = Duration::from_micros(16_667); // ~60 Hz
    let mut recycled_snapshot = RenderSnapshot::default();
    let mut next_tick = Instant::now();
    let mut ready_command = None;
    // Coalesced controls survive wakes until the next tick or structural-edit snapshot.
    let mut pending_speed = None;
    let mut pending_camera_aabb = None;
    let mut pending_cell_overlay_visible = None;

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
                Ok(SimCommand::SetCellOverlayVisible(visible)) => {
                    commands_processed += 1;
                    pending_cell_overlay_visible = Some(visible);
                }
                Ok(SimCommand::PrepareCellChunk {
                    chunk,
                    world_generation,
                    completion,
                }) => {
                    commands_processed += 1;
                    {
                        let mut core = core.lock().expect("simulation core lock poisoned");
                        record_crash_phase_for_core(&core, "cell chunk preparation");
                        run_sim_phase("cell chunk preparation", || {
                            core.prepare_requested_cell_chunk(chunk, world_generation);
                        });
                    }
                    let _ = completion.try_send(());
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
                    completion,
                }) => {
                    commands_processed += 1;
                    add_road_commands += 1;
                    let road_total = Instant::now();
                    let mut edit_metrics =
                        crate::nodes::sim::benchmark::road_edit::RoadEditMetrics::default();
                    edit_metrics.queue_wait_ms = enqueued_at.elapsed().as_secs_f64() * 1000.0;
                    let lock_wait_start = Instant::now();
                    let road_lock_wait_ms;
                    let outcome = {
                        let mut c = core.lock().expect("simulation core lock poisoned");
                        road_lock_wait_ms = lock_wait_start.elapsed().as_secs_f64() * 1000.0;
                        edit_metrics.lock_wait_ms = road_lock_wait_ms;
                        record_crash_command_for_core(
                            &c,
                            CrashCommand::AddRoad {
                                point_count: points.len(),
                                fwd_lanes,
                                bkw_lanes,
                                snap_to_existing_roads,
                            },
                        );
                        c.commit_road(
                            RoadCommitRequest {
                                points,
                                fwd_lanes,
                                bkw_lanes,
                                snap_to_existing_roads,
                                edit_plan,
                            },
                            road_total,
                            edit_metrics,
                        )
                    };
                    if let Some((preview_context, query_snapshot)) = outcome.road_snapshots {
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
                            outcome.add_internal_ms,
                            outcome.finalize_ms,
                            outcome.surface_ms,
                            outcome.mesh_ms,
                            outcome.snapshot_ms,
                            outcome.collect_refined_ms,
                            outcome.invalidated_refined_cache_entries
                        );
                    }
                    let _ = completion.try_send((outcome.committed, outcome.rejection));
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
        let mut step = FrameStepReport::default();
        let snapshot_ms: f64;
        let cell_prepare_ms: f64;
        let lock_held_ms: f64;
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
            if let Some(visible) = pending_cell_overlay_visible.take() {
                core.cell_overlay_visible = visible;
            }
            record_crash_phase_for_core(&core, "sim frame");

            // Publish completed edits immediately, even between movement deadlines. Otherwise
            // network/terrain dirty flags would put the removed queue delay back on the renderer.
            if tick_due {
                step = core.step_frame(FRAME_DT_S);
            }

            // Edits and ticks publish with their shown cell chunks already regenerated.
            let cell_prepare_start = Instant::now();
            record_crash_phase_for_core(&core, "stale cell chunk preparation");
            run_sim_phase("stale cell chunk preparation", || {
                core.prepare_stale_cell_chunks_internal()
            });
            cell_prepare_ms = cell_prepare_start.elapsed().as_secs_f64() * 1000.0;

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
                step.elapsed_minutes,
                step.pending_spawns_executed,
                step.hourly_ticks,
                step.daily_ticks,
                commands_processed,
            );
        }
        let unaccounted_ms =
            (active_ms - command_ms - lock_wait_ms - lock_held_ms - snapshot_write_ms).max(0.0);
        if perf_enabled && (active_ms >= 8.0 || command_ms >= 8.0 || step.elapsed_minutes > 0) {
            println!(
                "[DEBUG:perf] sim_frame active_ms={:.3} command_ms={:.3} lock_wait_ms={:.3} lock_held_ms={:.3} pathing_ms={:.3} agent_ms={:.3} minute_ms={:.3} pending_spawn_ms={:.3} hourly_ms={:.3} daily_ms={:.3} cell_prepare_ms={:.3} snapshot_ms={:.3} snapshot_write_ms={:.3} unaccounted_ms={:.3} elapsed_minutes={} pending_spawns={} hourly_ticks={} daily_ticks={} agents={} pathfinds={} commands={} set_speed_cmds={} camera_aabb_cmds={} add_road_cmds={} undo_cmds={} bulldoze_cmds={}",
                active_ms,
                command_ms,
                lock_wait_ms,
                lock_held_ms,
                step.pathing_ms,
                step.agent_ms,
                step.minute_ms,
                step.pending_spawn_ms,
                step.hourly_ms,
                step.daily_ms,
                cell_prepare_ms,
                snapshot_ms,
                snapshot_write_ms,
                unaccounted_ms,
                step.elapsed_minutes,
                step.pending_spawns_executed,
                step.hourly_ticks,
                step.daily_ticks,
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
