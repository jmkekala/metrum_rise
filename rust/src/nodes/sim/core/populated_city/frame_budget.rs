// SPDX-License-Identifier: GPL-2.0-only

//! `PERF-07` worst sim-frame benchmark (`docs/simulation.md#frame-budget-perf-07`).
//!
//! Replays one game day of a saved populated city frame by frame and times what the sim thread
//! does while it holds the core mutex: the frame step, stale cell-chunk preparation and the
//! render snapshot. Godot's upload acknowledgements run between frames, outside the timing, as
//! the main thread does. Run serially through `tools/sim02_gate.py`.
//!
//! Each frame also records the largest run-queue wait one of the frame's threads gained during
//! it, read outside the timing (`preemption`), so the report separates frames that another task
//! preempted from the frame's own work.

use super::super::frame::{FRAME_DT_S, FrameStepReport};
use super::super::snapshot::RenderSnapshot;
use super::super::state::SimCore;
use super::contract::{cache_dir, residents, timing_mode};
use super::snapshot;
use godot::prelude::Vector2;
use preemption::RunQueueWaits;
use std::time::Instant;

mod lane_entry_audit;
mod preemption;
// Kept out of `populated_city.rs`, whose bytes are part of the snapshot fixture revision.
mod snapshot_cull;

const BUDGET_MS: f64 = 8.0;
const FRAME_MS: f64 = 1000.0 / 60.0;
// A street-level camera box centred on the city; agents outside it are culled from snapshots.
const CAMERA_HALF_SPAN_M: f32 = 1000.0;
const WORST_FRAMES: usize = 12;
const MINUTES_PER_DAY: u64 = 24 * 60;
// A frame in which one of its threads gained this much run-queue wait counts as preempted by
// another task; the report's wait classes show where frames split
// (docs/simulation.md#frame-budget-perf-07).
const PREEMPTED_WAIT_MS: f64 = 0.5;
// Upper bounds of the report's wait classes, in milliseconds; the last class is unbounded.
const WAIT_CLASSES_MS: [f64; 7] = [0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0];

#[derive(Clone, Copy)]
struct FrameSample {
    ms: f64,
    day: u32,
    minute: u16,
    step: FrameStepReport,
    cell_ms: f64,
    snapshot_ms: f64,
    // The largest run-queue wait one of the frame's threads gained during the frame.
    wait_ms: f64,
}

impl FrameSample {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "ms":self.ms, "day":self.day, "minute":self.minute,
            "pathing_ms":self.step.pathing_ms, "agent_ms":self.step.agent_ms,
            "minute_ms":self.step.minute_ms, "pending_spawn_ms":self.step.pending_spawn_ms,
            "hourly_ms":self.step.hourly_ms, "daily_ms":self.step.daily_ms,
            "cell_ms":self.cell_ms, "snapshot_ms":self.snapshot_ms, "wait_ms":self.wait_ms,
            "elapsed_minutes":self.step.elapsed_minutes,
            "hourly_ticks":self.step.hourly_ticks, "daily_ticks":self.step.daily_ticks,
        })
    }

    fn preempted(&self) -> bool {
        self.wait_ms >= PREEMPTED_WAIT_MS
    }
}

fn absolute_minute(core: &SimCore) -> u64 {
    u64::from(core.time.day_index.saturating_sub(1)) * MINUTES_PER_DAY
        + u64::from(core.time.minute_of_day)
}

// What the Godot main thread does after uploading a published snapshot.
fn acknowledge_uploads(core: &mut SimCore, published: &RenderSnapshot) {
    if published.terrain_dirty {
        core.acknowledge_terrain_render_patches(&published.terrain_dirty_patch_states);
    }
    if published.water_dirty {
        for &(patch_x, patch_z, generation) in published.water_dirty_patch_states.iter() {
            core.watermap
                .acknowledge_render_patch(patch_x, patch_z, generation);
        }
        core.water_dirty = !core.watermap.dirty_render_patches().is_empty();
    }
    if published.network_dirty {
        core.acknowledge_network_render_generation(published.network_generation);
    }
}

// Frames by the largest wait one of their threads gained: count, mean and largest frame, and
// frames over the budget, for each class of `WAIT_CLASSES_MS`.
fn wait_classes(samples: &[FrameSample]) -> Vec<serde_json::Value> {
    let mut classes = vec![(0_usize, 0.0, 0.0_f64, 0_usize); WAIT_CLASSES_MS.len() + 1];
    for sample in samples {
        let class = WAIT_CLASSES_MS.partition_point(|&bound| bound <= sample.wait_ms);
        let (count, sum, max, over) = &mut classes[class];
        *count += 1;
        *sum += sample.ms;
        *max = max.max(sample.ms);
        *over += usize::from(sample.ms > BUDGET_MS);
    }
    classes
        .iter()
        .enumerate()
        .map(|(class, &(count, sum, max, over))| {
            serde_json::json!({
                "wait_below_ms": WAIT_CLASSES_MS.get(class),
                "frames": count, "mean_ms": sum / count.max(1) as f64, "max_ms": max,
                "over_8_ms": over,
            })
        })
        .collect()
}

// Nearest-rank percentile of an ascending sample.
fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    let rank = ((fraction * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    sorted[rank - 1]
}

#[test]
#[ignore = "PERF-07 release worst-frame benchmark over one game day of a prepared snapshot"]
fn perf07_frame_budget() {
    timing_mode();
    let speed: f32 = std::env::var("METRUM_PERF07_SPEED")
        .unwrap_or_else(|_| "32".into())
        .parse()
        .expect("METRUM_PERF07_SPEED must be a number");
    let load = Instant::now();
    let mut city = snapshot::load_snapshot(residents(), &cache_dir());
    let load_s = load.elapsed().as_secs_f64();
    let core = &mut city.core;
    core.time.speed_multiplier = speed;
    core.vehicle_ground_full_detail_m = std::env::var("METRUM_VEHICLE_GROUND_FULL_DETAIL_M")
        .map(|value| {
            value
                .parse()
                .expect("METRUM_VEHICLE_GROUND_FULL_DETAIL_M must be a number")
        })
        .unwrap_or(crate::nodes::sim::render::vehicle_ground::VEHICLE_GROUND_FULL_DETAIL_M);
    core.render_camera_position = Some(Vector2::ZERO);
    core.camera_aabb = (
        -CAMERA_HALF_SPAN_M,
        CAMERA_HALF_SPAN_M,
        -CAMERA_HALF_SPAN_M,
        CAMERA_HALF_SPAN_M,
    );
    // The first snapshot after loading uploads the whole world; Godot acknowledges it before
    // the measured day starts.
    let mut published = core.build_snapshot_reusing(RenderSnapshot::default());
    acknowledge_uploads(core, &published);
    let mut spare = RenderSnapshot::default();

    // A shorter span is a diagnostic only; acceptance runs the whole day.
    let minutes: u64 = std::env::var("METRUM_PERF07_MINUTES")
        .map(|value| {
            value
                .parse()
                .expect("METRUM_PERF07_MINUTES must be an integer")
        })
        .unwrap_or(MINUTES_PER_DAY);
    let end_minute = absolute_minute(core) + minutes;
    let frames_per_day = (MINUTES_PER_DAY as f64 * core.time.seconds_per_minute()
        / (FRAME_DT_S * f64::from(speed)))
    .ceil() as usize;
    let mut samples = Vec::with_capacity(frames_per_day + 1);
    // `METRUM_PERF07_SCHEDSTAT=0` skips the wait reads, to measure their overhead against the
    // same build; the report then watches no thread and counts no frame as preempted.
    let mut waits = if std::env::var("METRUM_PERF07_SCHEDSTAT").is_ok_and(|value| value == "0") {
        RunQueueWaits::unwatched()
    } else {
        RunQueueWaits::record_pool_and_caller()
    };
    let started = Instant::now();
    while absolute_minute(core) < end_minute {
        // The wait counters are read outside the timed span.
        waits.start();
        let frame_start = Instant::now();
        let step = core.step_frame(FRAME_DT_S);
        let cell_start = Instant::now();
        core.prepare_stale_cell_chunks_internal();
        let snapshot_start = Instant::now();
        let next = core.build_snapshot_reusing(std::mem::take(&mut spare));
        let frame_end = Instant::now();
        let wait_ms = waits.largest_growth_ms();
        spare = std::mem::replace(&mut published, next);
        acknowledge_uploads(core, &published);
        samples.push(FrameSample {
            ms: (frame_end - frame_start).as_secs_f64() * 1000.0,
            day: core.time.day_index,
            minute: core.time.minute_of_day,
            step,
            cell_ms: (snapshot_start - cell_start).as_secs_f64() * 1000.0,
            snapshot_ms: (frame_end - snapshot_start).as_secs_f64() * 1000.0,
            wait_ms,
        });
    }
    let run_s = started.elapsed().as_secs_f64();

    let mut sorted: Vec<f64> = samples.iter().map(|sample| sample.ms).collect();
    sorted.sort_by(f64::total_cmp);
    let mut worst = samples.clone();
    worst.sort_by(|a, b| b.ms.total_cmp(&a.ms));
    worst.truncate(WORST_FRAMES);
    let mut worst_unpreempted: Vec<FrameSample> = samples
        .iter()
        .filter(|sample| !sample.preempted())
        .copied()
        .collect();
    worst_unpreempted.sort_by(|a, b| b.ms.total_cmp(&a.ms));
    worst_unpreempted.truncate(WORST_FRAMES);
    let preempted = samples.iter().filter(|sample| sample.preempted());
    let over_budget = |preempted_frames: bool| {
        samples
            .iter()
            .filter(|sample| sample.preempted() == preempted_frames && sample.ms > BUDGET_MS)
            .count()
    };
    let boundary: Vec<f64> = samples
        .iter()
        .filter(|sample| sample.step.hourly_ticks > 0)
        .map(|sample| sample.ms)
        .collect();
    let plain_max = samples
        .iter()
        .filter(|sample| sample.step.hourly_ticks == 0)
        .map(|sample| sample.ms)
        .fold(0.0, f64::max);
    let component_max =
        |value: fn(&FrameSample) -> f64| samples.iter().map(value).fold(0.0, f64::max);
    let component_mean = |value: fn(&FrameSample) -> f64| {
        samples.iter().map(value).sum::<f64>() / samples.len() as f64
    };
    let mut snapshot_sorted: Vec<f64> = samples.iter().map(|sample| sample.snapshot_ms).collect();
    snapshot_sorted.sort_by(f64::total_cmp);
    eprintln!(
        "PERF07_DAY {}",
        serde_json::json!({
            "residents":residents(), "speed":speed, "load_s":load_s, "run_s":run_s,
            "vehicle_ground_full_detail_m":core.vehicle_ground_full_detail_m,
            "frames":samples.len(), "max_ms":sorted.last().copied().unwrap_or(0.0),
            "p99_ms":percentile(&sorted, 0.99), "p999_ms":percentile(&sorted, 0.999),
            "mean_ms":sorted.iter().sum::<f64>() / sorted.len() as f64,
            "over_8_ms":sorted.iter().filter(|&&ms| ms > BUDGET_MS).count(),
            "over_16_7_ms":sorted.iter().filter(|&&ms| ms > FRAME_MS).count(),
            "preemption_threads":waits.len(), "preempted_wait_ms":PREEMPTED_WAIT_MS,
            "preempted_frames":preempted.clone().count(),
            "max_unpreempted_ms":worst_unpreempted.first().map_or(0.0, |sample| sample.ms),
            "over_8_ms_preempted":over_budget(true), "over_8_ms_unpreempted":over_budget(false),
            "max_preempted_less_wait_ms":preempted
                .map(|sample| sample.ms - sample.wait_ms)
                .fold(0.0, f64::max),
            "wait_classes":wait_classes(&samples),
            "boundary_frames":boundary.len(),
            "boundary_max_ms":boundary.iter().copied().fold(0.0, f64::max),
            "non_boundary_max_ms":plain_max,
            "max_agent_ms":component_max(|sample| sample.step.agent_ms),
            "max_pathing_ms":component_max(|sample| sample.step.pathing_ms),
            "max_cell_ms":component_max(|sample| sample.cell_ms),
            "max_snapshot_ms":component_max(|sample| sample.snapshot_ms),
            "mean_snapshot_ms":snapshot_sorted.iter().sum::<f64>() / snapshot_sorted.len() as f64,
            "mean_agent_ms":component_mean(|sample| sample.step.agent_ms),
            "mean_slot_ms":component_mean(|sample| sample.step.minute_ms),
            "mean_pathing_ms":component_mean(|sample| sample.step.pathing_ms),
            "p99_snapshot_ms":percentile(&snapshot_sorted, 0.99),
            "worst":worst.iter().map(FrameSample::json).collect::<Vec<_>>(),
            "worst_unpreempted":worst_unpreempted.iter().map(FrameSample::json).collect::<Vec<_>>(),
            "record":format!("{:?}", city.record()),
        })
    );
}

/// Diagnostic, not acceptance timing: the hourly work of each of the hour's first four slots of a
/// prepared snapshot over `METRUM_PLAN_HOURS` hours (`SIM-02-I`: slot 0 publishes the census,
/// runs the operational hour and plans homes; slots 1 to 3 plan commerce and industry and
/// decide the hour's actions). Each frame crosses at most one slot. Prints each slot's mean and
/// worst milliseconds, and the whole frame's for slots 0 to 4, where the demand queues
/// (`SIM-02-J`) start the hour's level changes and removals.
#[test]
#[ignore = "SIM-02-I planning slot costs on a prepared snapshot; run alone, release"]
fn perf07_planning_slots() {
    timing_mode();
    let hours: u64 = std::env::var("METRUM_PLAN_HOURS")
        .unwrap_or_else(|_| "6".into())
        .parse()
        .expect("METRUM_PLAN_HOURS must be an integer");
    let mut city = snapshot::load_snapshot(residents(), &cache_dir());
    let core = &mut city.core;
    core.time.speed_multiplier = 1.0;
    let per_hour = crate::simulation::core::scheduler::SLOTS_PER_HOUR;
    // Warm the rest of this hour and one whole hour.
    let warm = core.time.absolute_slot() / per_hour + 2;
    while core.time.absolute_slot() / per_hour < warm {
        core.step_frame(FRAME_DT_S);
    }
    let end = warm + hours;
    let mut costs: [Vec<f64>; 4] = Default::default();
    let mut frames: [Vec<f64>; 5] = Default::default();
    let mut before = core.time.absolute_slot();
    while core.time.absolute_slot() / per_hour < end {
        let started = std::time::Instant::now();
        let step = core.step_frame(FRAME_DT_S / 16.0);
        let frame_ms = started.elapsed().as_secs_f64() * 1000.0;
        let now = core.time.absolute_slot();
        assert!(now <= before + 1, "a frame crossed more than one slot");
        if now != before && now % per_hour < 4 {
            costs[(now % per_hour) as usize].push(step.hourly_ms);
        }
        if now != before && now % per_hour < 5 {
            frames[(now % per_hour) as usize].push(frame_ms);
        }
        before = now;
    }
    let summary = |samples: &[Vec<f64>]| {
        samples
            .iter()
            .map(|slot| {
                serde_json::json!({
                    "mean": slot.iter().sum::<f64>() / slot.len().max(1) as f64,
                    "max": slot.iter().copied().fold(0.0, f64::max),
                    "samples": slot.len(),
                })
            })
            .collect::<Vec<_>>()
    };
    eprintln!(
        "PERF07_PLANNING_SLOTS {}",
        serde_json::json!({
            "residents": residents(),
            "hours": hours,
            "slots_ms": summary(&costs),
            "frames_ms": summary(&frames),
        })
    );
}

/// Diagnostic, not acceptance timing: the frame's flow-field drain on a prepared snapshot over
/// `METRUM_FLOW_HOURS` game hours at `METRUM_PERF07_SPEED` (`SIM-02-I`). Prints, per game hour,
/// the zones the drain found marked, the fields it built and those it left as they were, and
/// the frames' pathing milliseconds (CCH rebuild, index repair and field builds): the mean,
/// the mean of frames that built a field, and the worst.
#[test]
#[ignore = "SIM-02-I flow-field drain counts on a prepared snapshot; run alone, release"]
fn perf07_flow_field_drain() {
    use crate::debug::work::{Work, snapshot as work_snapshot};
    timing_mode();
    let hours: u64 = std::env::var("METRUM_FLOW_HOURS")
        .unwrap_or_else(|_| "4".into())
        .parse()
        .expect("METRUM_FLOW_HOURS must be an integer");
    let speed: f32 = std::env::var("METRUM_PERF07_SPEED")
        .unwrap_or_else(|_| "1".into())
        .parse()
        .expect("METRUM_PERF07_SPEED must be a number");
    let mut city = snapshot::load_snapshot(residents(), &cache_dir());
    let core = &mut city.core;
    core.time.speed_multiplier = speed;
    let per_hour = crate::simulation::core::scheduler::SLOTS_PER_HOUR;
    // The first frame after loading builds every field; the measured hours start at the next.
    core.step_frame(FRAME_DT_S);
    let first = core.time.absolute_slot() / per_hour + 1;
    while core.time.absolute_slot() / per_hour < first {
        core.step_frame(FRAME_DT_S);
    }
    let count = |counts: &[u64], work: Work| counts[work as usize];
    let mut report = Vec::new();
    for hour in first..first + hours {
        let before = work_snapshot();
        let mut pathing = Vec::new();
        let mut building = Vec::new();
        // Frames that start with a stale zone index, a stale entrance cache or a CCH rebuild
        // due, and their pathing milliseconds.
        let mut stale = [Vec::new(), Vec::new(), Vec::new()];
        while core.time.absolute_slot() / per_hour <= hour {
            let built = count(&work_snapshot(), Work::FlowFieldsBuilt);
            let allocator = &core.allocator;
            let due = [
                allocator.vacancy_index_stale(),
                allocator.entrances_dirty || allocator.entrances.len() != allocator.buildings.len(),
                !core.transit_network.cch_dirty_chunks.is_empty(),
            ];
            let step = core.step_frame(FRAME_DT_S);
            pathing.push(step.pathing_ms);
            if count(&work_snapshot(), Work::FlowFieldsBuilt) > built {
                building.push(step.pathing_ms);
            }
            for (frames, due) in stale.iter_mut().zip(due) {
                if due {
                    frames.push(step.pathing_ms);
                }
            }
        }
        let after = work_snapshot();
        let delta = |work| count(&after, work) - count(&before, work);
        let mean = |values: &[f64]| values.iter().sum::<f64>() / values.len().max(1) as f64;
        report.push(serde_json::json!({
            "hour": hour,
            "frames": pathing.len(),
            "zones_drained": delta(Work::FlowFieldZonesDrained),
            "fields_built": delta(Work::FlowFieldsBuilt),
            "fields_skipped": delta(Work::FlowFieldsSkipped),
            "buildings_placed": delta(Work::BuildingsPlaced),
            "buildings_removed": delta(Work::BuildingsRemoved),
            "pathing_mean_ms": mean(&pathing),
            "pathing_building_mean_ms": mean(&building),
            "pathing_max_ms": pathing.iter().copied().fold(0.0, f64::max),
            "stale_index_frames": stale[0].len(),
            "stale_index_mean_ms": mean(&stale[0]),
            "stale_entrance_frames": stale[1].len(),
            "stale_entrance_mean_ms": mean(&stale[1]),
            "cch_rebuild_frames": stale[2].len(),
            "cch_rebuild_mean_ms": mean(&stale[2]),
        }));
    }
    eprintln!(
        "PERF07_FLOW_FIELDS {}",
        serde_json::json!({
            "residents": residents(),
            "speed": speed,
            "nodes": core.region_graph.node_count(),
            "edges": core.region_graph.edge_count(),
            "hours": report,
        })
    );
}
