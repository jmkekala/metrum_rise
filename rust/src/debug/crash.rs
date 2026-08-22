//! Release-safe crash diagnostics and fixed-size flight recorder.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once, OnceLock, TryLockError};
use std::time::Instant;

const CRASH_RECORDER_CAPACITY: usize = 4096;

/// Crash diagnostics flag — set by `METRUM_CRASH_DIAGNOSTICS=1`.
pub static CRASH_DIAGNOSTICS_ENABLED: AtomicBool = AtomicBool::new(false);

static CRASH_RECORDER: OnceLock<Mutex<CrashRecorder>> = OnceLock::new();
static CRASH_LOG_DIR: OnceLock<PathBuf> = OnceLock::new();
static CRASH_START: OnceLock<Instant> = OnceLock::new();
static PANIC_HOOK: Once = Once::new();
static CRASH_DUMP_WRITTEN: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
struct CrashEvent {
    sequence: u64,
    elapsed_ms: u128,
    kind: CrashEventKind,
    summary: CrashSimSnapshot,
}

#[derive(Clone, Copy)]
struct CrashRecorder {
    events: [Option<CrashEvent>; CRASH_RECORDER_CAPACITY],
    next: usize,
    len: usize,
    sequence: u64,
}

impl CrashRecorder {
    fn new() -> Self {
        Self {
            events: [None; CRASH_RECORDER_CAPACITY],
            next: 0,
            len: 0,
            sequence: 0,
        }
    }

    fn push(&mut self, kind: CrashEventKind, summary: CrashSimSnapshot) {
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
    },
    /// Undo command from Godot.
    Undo,
    /// Bulldoze command from Godot.
    Bulldoze,
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
    install_panic_hook();
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
        recorder.push(kind, summary);
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
    writeln!(out, "flight_recorder_capacity={CRASH_RECORDER_CAPACITY}")?;
    match CRASH_RECORDER.get() {
        Some(recorder) => match recorder.try_lock() {
            Ok(recorder) => recorder.write_events(&mut out)?,
            Err(TryLockError::Poisoned(poisoned)) => {
                poisoned.into_inner().write_events(&mut out)?
            }
            Err(TryLockError::WouldBlock) => {
                writeln!(
                    out,
                    "flight_recorder_unavailable=locked_by_panicking_thread"
                )?;
            }
        },
        None => {
            writeln!(out, "flight_recorder_unavailable=not_initialized")?;
        }
    }
    writeln!(out)?;
    writeln!(out, "backtrace:")?;
    writeln!(out, "{:?}", std::backtrace::Backtrace::force_capture())?;
    out.flush()?;
    eprintln!("[CRASH_DIAGNOSTICS] wrote crash dump to {}", path.display());
    Ok(path)
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
        } => write!(
            out,
            "add_road points={point_count} fwd_lanes={fwd_lanes} bkw_lanes={bkw_lanes} snap={snap_to_existing_roads}"
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
