// SPDX-License-Identifier: GPL-2.0-only

//! `PERF-07` worst sim-frame benchmark (`docs/simulation.md#frame-budget-perf-07`).
//!
//! Replays one game day of a saved populated city frame by frame and times what the sim thread
//! does while it holds the core mutex: the frame step, stale cell-chunk preparation and the
//! render snapshot. Godot's upload acknowledgements run between frames, outside the timing, as
//! the main thread does. Run serially through `tools/sim02_gate.py`.

use super::super::frame::{FRAME_DT_S, FrameStepReport};
use super::super::snapshot::RenderSnapshot;
use super::super::state::SimCore;
use super::contract::{cache_dir, residents, timing_mode};
use super::snapshot;
use std::time::Instant;

const BUDGET_MS: f64 = 8.0;
const FRAME_MS: f64 = 1000.0 / 60.0;
// A street-level camera box centred on the city; agents outside it are culled from snapshots.
const CAMERA_HALF_SPAN_M: f32 = 1000.0;
const WORST_FRAMES: usize = 12;
const MINUTES_PER_DAY: u64 = 24 * 60;

#[derive(Clone, Copy)]
struct FrameSample {
    ms: f64,
    day: u32,
    minute: u16,
    step: FrameStepReport,
    cell_ms: f64,
    snapshot_ms: f64,
}

impl FrameSample {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "ms":self.ms, "day":self.day, "minute":self.minute,
            "pathing_ms":self.step.pathing_ms, "agent_ms":self.step.agent_ms,
            "minute_ms":self.step.minute_ms, "pending_spawn_ms":self.step.pending_spawn_ms,
            "hourly_ms":self.step.hourly_ms, "daily_ms":self.step.daily_ms,
            "cell_ms":self.cell_ms, "snapshot_ms":self.snapshot_ms,
            "elapsed_minutes":self.step.elapsed_minutes,
            "hourly_ticks":self.step.hourly_ticks, "daily_ticks":self.step.daily_ticks,
        })
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

    let end_minute = absolute_minute(core) + MINUTES_PER_DAY;
    let frames_per_day = (MINUTES_PER_DAY as f64 * core.time.seconds_per_minute()
        / (FRAME_DT_S * f64::from(speed)))
    .ceil() as usize;
    let mut samples = Vec::with_capacity(frames_per_day + 1);
    let started = Instant::now();
    while absolute_minute(core) < end_minute {
        let frame_start = Instant::now();
        let step = core.step_frame(FRAME_DT_S);
        let cell_start = Instant::now();
        core.prepare_stale_cell_chunks_internal();
        let snapshot_start = Instant::now();
        let next = core.build_snapshot_reusing(std::mem::take(&mut spare));
        let frame_end = Instant::now();
        spare = std::mem::replace(&mut published, next);
        acknowledge_uploads(core, &published);
        samples.push(FrameSample {
            ms: (frame_end - frame_start).as_secs_f64() * 1000.0,
            day: core.time.day_index,
            minute: core.time.minute_of_day,
            step,
            cell_ms: (snapshot_start - cell_start).as_secs_f64() * 1000.0,
            snapshot_ms: (frame_end - snapshot_start).as_secs_f64() * 1000.0,
        });
    }
    let run_s = started.elapsed().as_secs_f64();

    let mut sorted: Vec<f64> = samples.iter().map(|sample| sample.ms).collect();
    sorted.sort_by(f64::total_cmp);
    let mut worst = samples.clone();
    worst.sort_by(|a, b| b.ms.total_cmp(&a.ms));
    worst.truncate(WORST_FRAMES);
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
    eprintln!(
        "PERF07_DAY {}",
        serde_json::json!({
            "residents":residents(), "speed":speed, "load_s":load_s, "run_s":run_s,
            "frames":samples.len(), "max_ms":sorted.last().copied().unwrap_or(0.0),
            "p99_ms":percentile(&sorted, 0.99), "p999_ms":percentile(&sorted, 0.999),
            "mean_ms":sorted.iter().sum::<f64>() / sorted.len() as f64,
            "over_8_ms":sorted.iter().filter(|&&ms| ms > BUDGET_MS).count(),
            "over_16_7_ms":sorted.iter().filter(|&&ms| ms > FRAME_MS).count(),
            "boundary_frames":boundary.len(),
            "boundary_max_ms":boundary.iter().copied().fold(0.0, f64::max),
            "non_boundary_max_ms":plain_max,
            "max_agent_ms":component_max(|sample| sample.step.agent_ms),
            "max_pathing_ms":component_max(|sample| sample.step.pathing_ms),
            "max_cell_ms":component_max(|sample| sample.cell_ms),
            "max_snapshot_ms":component_max(|sample| sample.snapshot_ms),
            "worst":worst.iter().map(FrameSample::json).collect::<Vec<_>>(),
            "record":format!("{:?}", city.record()),
        })
    );
}
