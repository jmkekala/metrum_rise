// SPDX-License-Identifier: GPL-2.0-only

//! Release-safe crash diagnostics and fixed-size flight recorder.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Mutex, Once, OnceLock, PoisonError, TryLockError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use godot::prelude::Vector3;

const CRASH_RECORDER_CAPACITY: usize = 4096;
const DEFAULT_HANG_WATCHDOG_TIMEOUT_MS: u64 = 10_000;
const HANG_WATCHDOG_POLL_MS: u64 = 500;

/// Crash diagnostics flag — set by `METRUM_CRASH_DIAGNOSTICS=1`.
pub static CRASH_DIAGNOSTICS_ENABLED: AtomicBool = AtomicBool::new(false);

static CRASH_RECORDER: OnceLock<Mutex<CrashRecorder>> = OnceLock::new();
static CRASH_LOG_DIR: OnceLock<PathBuf> = OnceLock::new();
static CRASH_START: OnceLock<Instant> = OnceLock::new();
static PANIC_HOOK: Once = Once::new();
// The running watchdog: dropping the sender stops it, then the handle joins it.
static HANG_WATCHDOG: Mutex<Option<(mpsc::Sender<()>, JoinHandle<()>)>> = Mutex::new(None);
static CRASH_DUMP_WRITTEN: AtomicBool = AtomicBool::new(false);
static HANG_DUMP_WRITTEN: AtomicBool = AtomicBool::new(false);
static WATCHDOG_MONITOR_ACTIVE: AtomicBool = AtomicBool::new(false);
static WATCHDOG_ABORT_AFTER_DUMP: AtomicBool = AtomicBool::new(false);
static WATCHDOG_LAST_PROGRESS_MS: AtomicU64 = AtomicU64::new(0);
static WATCHDOG_PROGRESS_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static WATCHDOG_STATE: OnceLock<Mutex<WatchdogState>> = OnceLock::new();

#[derive(Clone, Copy)]
struct CrashEvent {
    sequence: u64,
    elapsed_ms: u128,
    kind: CrashEventKind,
    summary: CrashSimSnapshot,
}

struct CrashRecorder {
    // Initialize directly on the heap: constructing/moving the fixed array made even
    // disabled initialization reserve more than a default test/Rayon thread's stack.
    events: Box<[Option<CrashEvent>]>,
    next: usize,
    len: usize,
    sequence: u64,
}

#[derive(Clone, Copy, Default)]
struct WatchdogState {
    last_event: Option<CrashEvent>,
}

impl CrashRecorder {
    fn new() -> Self {
        Self {
            events: vec![None; CRASH_RECORDER_CAPACITY].into_boxed_slice(),
            next: 0,
            len: 0,
            sequence: 0,
        }
    }

    fn push(&mut self, kind: CrashEventKind, summary: CrashSimSnapshot) -> CrashEvent {
        let event = CrashEvent {
            sequence: self.sequence,
            elapsed_ms: CRASH_START
                .get()
                .map(Instant::elapsed)
                .map_or(0, |d| d.as_millis()),
            kind,
            summary,
        };
        self.events[self.next] = Some(event);
        self.next = (self.next + 1) % CRASH_RECORDER_CAPACITY;
        self.len = (self.len + 1).min(CRASH_RECORDER_CAPACITY);
        self.sequence = self.sequence.wrapping_add(1);
        event
    }

    fn write_events(&self, out: &mut impl Write) -> std::io::Result<()> {
        let oldest = if self.len == CRASH_RECORDER_CAPACITY {
            self.next
        } else {
            0
        };
        for offset in 0..self.len {
            let index = (oldest + offset) % CRASH_RECORDER_CAPACITY;
            if let Some(event) = self.events[index] {
                write_crash_event(out, event)?;
            }
        }
        Ok(())
    }
}

/// Compact authoritative-state summary captured by the crash flight recorder.
#[derive(Clone, Copy)]
pub(crate) struct CrashSimSnapshot {
    /// Current 1-indexed operational day.
    pub(crate) day_index: u32,
    /// Current minute after operational midnight.
    pub(crate) minute_of_day: u16,
    /// Active simulation speed multiplier.
    pub(crate) speed_multiplier: f32,
    /// Total live agents in the authoritative SoA.
    pub(crate) agent_count: usize,
    /// Total pathfinding calls observed this session.
    pub(crate) pathfind_count: u32,
    /// Total allocated building records.
    pub(crate) building_count: usize,
    /// Total household records.
    pub(crate) household_count: usize,
    /// Road graph node slot count.
    pub(crate) road_node_count: usize,
    /// Road graph edge slot count.
    pub(crate) road_edge_count: usize,
    /// Published road-tool surface generation.
    pub(crate) road_generation: u64,
    /// Pending demand spawn queue length.
    pub(crate) pending_demand_spawns: usize,
    /// Last agent tick duration in microseconds.
    pub(crate) last_agent_tick_us: u64,
    /// Last daily economy tick duration in milliseconds.
    pub(crate) last_tick_duration_ms: f64,
    /// Whether terrain has unacknowledged render dirtiness.
    pub(crate) terrain_dirty: bool,
    /// Whether water has unacknowledged render dirtiness.
    pub(crate) water_dirty: bool,
    /// Whether network visuals have unacknowledged render dirtiness.
    pub(crate) network_dirty: bool,
}

/// Command marker captured by the crash flight recorder.
#[derive(Clone, Copy)]
pub(crate) enum CrashCommand {
    /// Speed multiplier command from Godot.
    SetSpeed {
        /// Requested simulation speed.
        speed: f32,
    },
    /// Camera culling bounds command from Godot.
    SetCameraAabb {
        /// Minimum world X.
        x_min: f32,
        /// Maximum world X.
        x_max: f32,
        /// Minimum world Z.
        z_min: f32,
        /// Maximum world Z.
        z_max: f32,
    },
    /// Road placement command from Godot.
    AddRoad {
        /// Polyline point count.
        point_count: usize,
        /// Forward lane count.
        fwd_lanes: i32,
        /// Backward lane count.
        bkw_lanes: i32,
        /// Whether endpoints may snap to existing roads.
        snap_to_existing_roads: bool,
        /// Fixed-size geometry identity for reproducing the submitted stroke.
        geometry: CrashRoadGeometry,
    },
    /// Undo command from Godot.
    Undo,
    /// Bulldoze command from Godot.
    Bulldoze,
}

/// Allocation-free geometry identity retained with an `AddRoad` flight-recorder event.
#[derive(Clone, Copy)]
pub(crate) struct CrashRoadGeometry {
    start: [f32; 3],
    end: [f32; 3],
    points_hash: u64,
}

impl CrashRoadGeometry {
    /// Captures both endpoints and an exact stable fingerprint of every submitted float bit.
    pub(crate) fn from_points(points: &[Vector3]) -> Self {
        let components = |point: Vector3| [point.x, point.y, point.z];
        let start = points.first().copied().map_or([0.0; 3], components);
        let end = points.last().copied().map_or([0.0; 3], components);

        // FNV-1a over an explicit little-endian stream is stable across processes and targets.
        // Length is included so an empty stroke cannot alias a one-point all-zero stroke.
        let mut points_hash = 0xcbf2_9ce4_8422_2325_u64;
        for byte in (points.len() as u64).to_le_bytes() {
            points_hash = (points_hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        for point in points {
            for component in components(*point) {
                for byte in component.to_bits().to_le_bytes() {
                    points_hash =
                        (points_hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
                }
            }
        }
        Self {
            start,
            end,
            points_hash,
        }
    }
}

#[derive(Clone, Copy)]
enum CrashEventKind {
    Phase {
        phase: &'static str,
    },
    Command {
        command: CrashCommand,
    },
    Frame {
        active_ms: f64,
        command_ms: f64,
        lock_wait_ms: f64,
        lock_held_ms: f64,
        snapshot_ms: f64,
        snapshot_write_ms: f64,
        elapsed_minutes: u16,
        pending_spawns_executed: usize,
        hourly_ticks: usize,
        daily_ticks: usize,
        commands_processed: usize,
    },
}

/// Reads crash-diagnostics environment variables and installs the panic hook when enabled.
pub(crate) fn init() {
    let _ = CRASH_START.set(Instant::now());
    let crash_on = std::env::var("METRUM_CRASH_DIAGNOSTICS")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false);
    CRASH_DIAGNOSTICS_ENABLED.store(crash_on, Ordering::Relaxed);
    if !crash_on {
        return;
    }

    let log_dir = std::env::var("METRUM_CRASH_LOG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("logs"));
    if let Err(err) = std::fs::create_dir_all(&log_dir) {
        eprintln!(
            "[CRASH_DIAGNOSTICS] could not create log directory '{}': {}",
            log_dir.display(),
            err
        );
    }
    let _ = CRASH_LOG_DIR.set(log_dir);
    let _ = CRASH_RECORDER.set(Mutex::new(CrashRecorder::new()));
    let _ = WATCHDOG_STATE.set(Mutex::new(WatchdogState::default()));
    install_panic_hook();
    if let Some(timeout_ms) = hang_watchdog_timeout_ms() {
        start_hang_watchdog(timeout_ms);
    }
    println!(
        "[DEBUG] Crash diagnostics enabled (METRUM_CRASH_DIAGNOSTICS=1, logs={})",
        crash_log_dir().display()
    );
}

/// Returns `true` if release-safe crash diagnostics are currently enabled.
#[inline(always)]
pub fn is_crash_diagnostics_enabled() -> bool {
    CRASH_DIAGNOSTICS_ENABLED.load(Ordering::Relaxed)
}

/// Records the current high-level simulation phase in the crash flight recorder.
pub(crate) fn record_crash_phase(phase: &'static str, summary: CrashSimSnapshot) {
    record_crash_event(CrashEventKind::Phase { phase }, summary);
}

/// Records an input command in the crash flight recorder.
pub(crate) fn record_crash_command(command: CrashCommand, summary: CrashSimSnapshot) {
    record_crash_event(CrashEventKind::Command { command }, summary);
}

/// Records a completed simulation frame in the crash flight recorder.
#[allow(clippy::too_many_arguments)]
pub(crate) fn record_crash_frame(
    summary: CrashSimSnapshot,
    active_ms: f64,
    command_ms: f64,
    lock_wait_ms: f64,
    lock_held_ms: f64,
    snapshot_ms: f64,
    snapshot_write_ms: f64,
    elapsed_minutes: u16,
    pending_spawns_executed: usize,
    hourly_ticks: usize,
    daily_ticks: usize,
    commands_processed: usize,
) {
    record_crash_event(
        CrashEventKind::Frame {
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
        },
        summary,
    );
}

/// Writes the flight recorder once for a caught fatal path.
pub(crate) fn flush_crash_diagnostics(reason: &str) {
    if !is_crash_diagnostics_enabled() {
        return;
    }
    let _ = write_crash_dump(reason, None);
}

/// Disarms hang detection after an orderly simulation-thread shutdown.
pub(crate) fn suspend_hang_watchdog() {
    WATCHDOG_MONITOR_ACTIVE.store(false, Ordering::Release);
}

/// Stops the hang watchdog and joins its thread, so it cannot run once the library unloads.
/// A later [`init`] may start it again.
pub(crate) fn shutdown() {
    let watchdog = HANG_WATCHDOG
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    if let Some((stop, thread)) = watchdog {
        drop(stop);
        let _ = thread.join();
    }
}

fn install_panic_hook() {
    PANIC_HOOK.call_once(|| {
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if CRASH_DIAGNOSTICS_ENABLED.load(Ordering::Relaxed) {
                let _ = write_crash_dump("panic", Some(info));
            }
            previous_hook(info);
        }));
    });
}

fn record_crash_event(kind: CrashEventKind, summary: CrashSimSnapshot) {
    if !is_crash_diagnostics_enabled() {
        return;
    }
    let Some(recorder) = CRASH_RECORDER.get() else {
        return;
    };
    if let Ok(mut recorder) = recorder.lock() {
        let event = recorder.push(kind, summary);
        drop(recorder);
        record_watchdog_progress(event);
    }
}

fn record_watchdog_progress(event: CrashEvent) {
    let elapsed_ms = event.elapsed_ms.min(u128::from(u64::MAX)) as u64;
    WATCHDOG_LAST_PROGRESS_MS.store(elapsed_ms, Ordering::Release);
    WATCHDOG_PROGRESS_SEQUENCE.store(event.sequence.saturating_add(1), Ordering::Release);
    if let Some(state) = WATCHDOG_STATE.get()
        && let Ok(mut state) = state.lock()
    {
        state.last_event = Some(event);
    }
    WATCHDOG_MONITOR_ACTIVE.store(true, Ordering::Release);
}

fn hang_watchdog_timeout_ms() -> Option<u64> {
    match std::env::var("METRUM_HANG_WATCHDOG_MS") {
        Ok(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed == "0" {
                None
            } else {
                trimmed.parse::<u64>().ok().filter(|value| *value > 0)
            }
        }
        Err(_) => Some(DEFAULT_HANG_WATCHDOG_TIMEOUT_MS),
    }
}

fn start_hang_watchdog(timeout_ms: u64) {
    let mut watchdog = HANG_WATCHDOG.lock().unwrap_or_else(PoisonError::into_inner);
    if watchdog.is_some() {
        return;
    }
    WATCHDOG_ABORT_AFTER_DUMP.store(
        std::env::var("METRUM_HANG_ABORT")
            .map(|value| !value.is_empty() && value != "0")
            .unwrap_or(false),
        Ordering::Relaxed,
    );
    let (stop, stopped) = mpsc::channel();
    let spawn_result = std::thread::Builder::new()
        .name("metrum-hang-watchdog".to_string())
        .spawn(move || run_hang_watchdog(timeout_ms, stopped));
    match spawn_result {
        Ok(thread) => *watchdog = Some((stop, thread)),
        Err(err) => eprintln!("[CRASH_DIAGNOSTICS] could not start hang watchdog: {err}"),
    }
}

fn run_hang_watchdog(timeout_ms: u64, stopped: mpsc::Receiver<()>) {
    let poll = Duration::from_millis(HANG_WATCHDOG_POLL_MS);
    loop {
        // Polls every interval until `shutdown` drops the sender.
        if stopped.recv_timeout(poll) != Err(RecvTimeoutError::Timeout) {
            return;
        }
        if !is_crash_diagnostics_enabled() {
            continue;
        }
        if WATCHDOG_PROGRESS_SEQUENCE.load(Ordering::Acquire) == 0 {
            continue;
        }
        if !WATCHDOG_MONITOR_ACTIVE.load(Ordering::Acquire) {
            continue;
        }
        let Some(start) = CRASH_START.get() else {
            continue;
        };
        let now_ms = start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        let last_progress_ms = WATCHDOG_LAST_PROGRESS_MS.load(Ordering::Acquire);
        let stalled_ms = now_ms.saturating_sub(last_progress_ms);
        if stalled_ms < timeout_ms {
            continue;
        }
        if HANG_DUMP_WRITTEN.swap(true, Ordering::AcqRel) {
            continue;
        }
        match write_hang_dump(stalled_ms, timeout_ms) {
            Ok(path) => {
                eprintln!("[CRASH_DIAGNOSTICS] watchdog hang dump: {}", path.display());
            }
            Err(err) => {
                eprintln!("[CRASH_DIAGNOSTICS] could not write watchdog hang dump: {err}");
            }
        }
        if WATCHDOG_ABORT_AFTER_DUMP.load(Ordering::Relaxed) {
            std::process::abort();
        }
    }
}

fn write_crash_dump(
    reason: &str,
    panic_info: Option<&std::panic::PanicHookInfo<'_>>,
) -> std::io::Result<PathBuf> {
    if CRASH_DUMP_WRITTEN.swap(true, Ordering::AcqRel) {
        return Ok(crash_log_dir().to_path_buf());
    }

    let log_dir = crash_log_dir();
    std::fs::create_dir_all(log_dir)?;
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S%.3f");
    let path = log_dir.join(format!(
        "metrum-crash-{}-pid{}.log",
        timestamp,
        std::process::id()
    ));
    let file = std::fs::File::create(&path)?;
    let mut out = std::io::BufWriter::new(file);

    writeln!(out, "Metrum Rise crash diagnostics")?;
    writeln!(out, "reason={reason}")?;
    writeln!(out, "timestamp={}", chrono::Local::now().to_rfc3339())?;
    writeln!(out, "pid={}", std::process::id())?;
    if let Some(info) = panic_info {
        write_panic_info(&mut out, info)?;
    }
    writeln!(out)?;
    write_flight_recorder(&mut out, "locked_by_panicking_thread")?;
    write_backtrace(&mut out, "backtrace")?;
    out.flush()?;
    eprintln!("[CRASH_DIAGNOSTICS] wrote crash dump to {}", path.display());
    Ok(path)
}

fn write_hang_dump(stalled_ms: u64, timeout_ms: u64) -> std::io::Result<PathBuf> {
    let log_dir = crash_log_dir();
    std::fs::create_dir_all(log_dir)?;
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S%.3f");
    let path = log_dir.join(format!(
        "metrum-hang-{}-pid{}.log",
        timestamp,
        std::process::id()
    ));
    let file = std::fs::File::create(&path)?;
    let mut out = std::io::BufWriter::new(file);

    writeln!(out, "Metrum Rise hang diagnostics")?;
    writeln!(out, "reason=watchdog_hang")?;
    writeln!(out, "timestamp={}", chrono::Local::now().to_rfc3339())?;
    writeln!(out, "pid={}", std::process::id())?;
    writeln!(out, "timeout_ms={timeout_ms}")?;
    writeln!(out, "stalled_ms={stalled_ms}")?;
    writeln!(
        out,
        "abort_after_dump={}",
        WATCHDOG_ABORT_AFTER_DUMP.load(Ordering::Relaxed)
    )?;
    write_watchdog_state(&mut out)?;
    writeln!(out)?;
    write_flight_recorder(&mut out, "locked_by_stalled_thread")?;
    write_backtrace(&mut out, "watchdog_backtrace")?;
    out.flush()?;
    Ok(path)
}

fn write_watchdog_state(out: &mut impl Write) -> std::io::Result<()> {
    match WATCHDOG_STATE.get() {
        Some(state) => match state.try_lock() {
            Ok(state) => match state.last_event {
                Some(event) => {
                    write!(out, "last_progress_event=")?;
                    write_crash_event(out, event)?;
                }
                None => {
                    writeln!(out, "last_progress_event=none")?;
                }
            },
            Err(TryLockError::Poisoned(poisoned)) => match poisoned.into_inner().last_event {
                Some(event) => {
                    write!(out, "last_progress_event=")?;
                    write_crash_event(out, event)?;
                }
                None => {
                    writeln!(out, "last_progress_event=none")?;
                }
            },
            Err(TryLockError::WouldBlock) => {
                writeln!(out, "watchdog_state_unavailable=locked")?;
            }
        },
        None => {
            writeln!(out, "watchdog_state_unavailable=not_initialized")?;
        }
    }
    Ok(())
}

fn write_flight_recorder(out: &mut impl Write, locked_reason: &str) -> std::io::Result<()> {
    writeln!(out, "flight_recorder_capacity={CRASH_RECORDER_CAPACITY}")?;
    match CRASH_RECORDER.get() {
        Some(recorder) => match recorder.try_lock() {
            Ok(recorder) => recorder.write_events(out)?,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner().write_events(out)?,
            Err(TryLockError::WouldBlock) => {
                writeln!(out, "flight_recorder_unavailable={locked_reason}")?;
            }
        },
        None => {
            writeln!(out, "flight_recorder_unavailable=not_initialized")?;
        }
    }
    Ok(())
}

fn write_backtrace(out: &mut impl Write, label: &str) -> std::io::Result<()> {
    writeln!(out)?;
    writeln!(out, "{label}:")?;
    writeln!(out, "{:?}", std::backtrace::Backtrace::force_capture())?;
    Ok(())
}

fn crash_log_dir() -> &'static Path {
    CRASH_LOG_DIR
        .get()
        .map(PathBuf::as_path)
        .unwrap_or_else(|| Path::new("logs"))
}

fn write_panic_info(
    out: &mut impl Write,
    info: &std::panic::PanicHookInfo<'_>,
) -> std::io::Result<()> {
    if let Some(location) = info.location() {
        writeln!(
            out,
            "panic_location={}:{}:{}",
            location.file(),
            location.line(),
            location.column()
        )?;
    }
    if let Some(message) = info.payload().downcast_ref::<&str>() {
        writeln!(out, "panic_payload={message}")?;
    } else if let Some(message) = info.payload().downcast_ref::<String>() {
        writeln!(out, "panic_payload={message}")?;
    } else {
        writeln!(out, "panic_payload=(non-string payload)")?;
    }
    let thread = std::thread::current();
    writeln!(out, "panic_thread={}", thread.name().unwrap_or("(unnamed)"))?;
    Ok(())
}

fn write_crash_event(out: &mut impl Write, event: CrashEvent) -> std::io::Result<()> {
    write!(out, "#{:06} +{}ms ", event.sequence, event.elapsed_ms)?;
    match event.kind {
        CrashEventKind::Phase { phase } => {
            write!(out, "phase={phase}")?;
        }
        CrashEventKind::Command { command } => {
            write!(out, "command=")?;
            write_crash_command(out, command)?;
        }
        CrashEventKind::Frame {
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
        } => {
            write!(
                out,
                "frame active_ms={active_ms:.3} command_ms={command_ms:.3} lock_wait_ms={lock_wait_ms:.3} lock_held_ms={lock_held_ms:.3} snapshot_ms={snapshot_ms:.3} snapshot_write_ms={snapshot_write_ms:.3} elapsed_minutes={elapsed_minutes} pending_spawns_executed={pending_spawns_executed} hourly_ticks={hourly_ticks} daily_ticks={daily_ticks} commands={commands_processed}"
            )?;
        }
    }
    write_crash_summary(out, event.summary)?;
    writeln!(out)?;
    Ok(())
}

fn write_crash_command(out: &mut impl Write, command: CrashCommand) -> std::io::Result<()> {
    match command {
        CrashCommand::SetSpeed { speed } => write!(out, "set_speed speed={speed:.2}"),
        CrashCommand::SetCameraAabb {
            x_min,
            x_max,
            z_min,
            z_max,
        } => write!(
            out,
            "set_camera_aabb x_min={x_min:.1} x_max={x_max:.1} z_min={z_min:.1} z_max={z_max:.1}"
        ),
        CrashCommand::AddRoad {
            point_count,
            fwd_lanes,
            bkw_lanes,
            snap_to_existing_roads,
            geometry,
        } => write!(
            out,
            "add_road points={point_count} start=({:.3},{:.3},{:.3}) end=({:.3},{:.3},{:.3}) points_hash={:016x} fwd_lanes={fwd_lanes} bkw_lanes={bkw_lanes} snap={snap_to_existing_roads}",
            geometry.start[0],
            geometry.start[1],
            geometry.start[2],
            geometry.end[0],
            geometry.end[1],
            geometry.end[2],
            geometry.points_hash,
        ),
        CrashCommand::Undo => write!(out, "undo"),
        CrashCommand::Bulldoze => write!(out, "bulldoze"),
    }
}

fn write_crash_summary(out: &mut impl Write, summary: CrashSimSnapshot) -> std::io::Result<()> {
    write!(
        out,
        " day={} time={:02}:{:02} speed={:.2} agents={} pathfinds={} buildings={} households={} road_nodes={} road_edges={} road_generation={} pending_demand_spawns={} last_agent_tick_us={} last_tick_ms={:.3} dirty=(terrain:{},water:{},network:{})",
        summary.day_index,
        summary.minute_of_day / 60,
        summary.minute_of_day % 60,
        summary.speed_multiplier,
        summary.agent_count,
        summary.pathfind_count,
        summary.building_count,
        summary.household_count,
        summary.road_node_count,
        summary.road_edge_count,
        summary.road_generation,
        summary.pending_demand_spawns,
        summary.last_agent_tick_us,
        summary.last_tick_duration_ms,
        summary.terrain_dirty,
        summary.water_dirty,
        summary.network_dirty,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // The extension calls `shutdown` before Godot unloads it: the watchdog thread must have
    // exited when it returns, and a later load must be able to start it again.
    #[test]
    fn watchdog_shutdown_joins_its_thread_and_allows_a_restart() {
        for _ in 0..2 {
            start_hang_watchdog(60_000);
            assert!(HANG_WATCHDOG.lock().unwrap().is_some());
            shutdown();
            assert!(HANG_WATCHDOG.lock().unwrap().is_none());
        }
    }

    #[test]
    fn flight_recorder_initialization_has_bounded_stack_storage() {
        assert!(std::mem::size_of::<CrashRecorder>() <= 64);
        let recorder = CrashRecorder::new();
        assert_eq!(recorder.events.len(), CRASH_RECORDER_CAPACITY);
        assert!(recorder.events.iter().all(Option::is_none));
    }

    #[test]
    fn road_command_geometry_is_fixed_size_stable_and_printed() {
        let points = [
            Vector3::new(1.25, -2.5, 3.75),
            Vector3::new(4.5, 5.25, -6.0),
            Vector3::new(7.0, 8.5, 9.25),
        ];
        let geometry = CrashRoadGeometry::from_points(&points);
        assert_eq!(geometry.start, [1.25, -2.5, 3.75]);
        assert_eq!(geometry.end, [7.0, 8.5, 9.25]);
        assert_eq!(geometry.points_hash, 0x7d9d_4eb7_7a4c_2803);
        assert!(std::mem::size_of::<CrashRoadGeometry>() <= 32);

        let mut rendered = Vec::new();
        write_crash_command(
            &mut rendered,
            CrashCommand::AddRoad {
                point_count: points.len(),
                fwd_lanes: 2,
                bkw_lanes: 1,
                snap_to_existing_roads: true,
                geometry,
            },
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(rendered).unwrap(),
            "add_road points=3 start=(1.250,-2.500,3.750) end=(7.000,8.500,9.250) points_hash=7d9d4eb77a4c2803 fwd_lanes=2 bkw_lanes=1 snap=true"
        );
    }

    #[test]
    fn road_command_hash_covers_interior_points_and_length() {
        let endpoints = [Vector3::ZERO, Vector3::new(8.0, 0.0, 0.0)];
        let with_midpoint = [
            Vector3::ZERO,
            Vector3::new(4.0, 0.0, 0.0),
            Vector3::new(8.0, 0.0, 0.0),
        ];
        let moved_midpoint = [
            Vector3::ZERO,
            Vector3::new(4.0, 0.0, 1.0),
            Vector3::new(8.0, 0.0, 0.0),
        ];
        assert_ne!(
            CrashRoadGeometry::from_points(&endpoints).points_hash,
            CrashRoadGeometry::from_points(&with_midpoint).points_hash
        );
        assert_ne!(
            CrashRoadGeometry::from_points(&with_midpoint).points_hash,
            CrashRoadGeometry::from_points(&moved_midpoint).points_hash
        );
    }
}
