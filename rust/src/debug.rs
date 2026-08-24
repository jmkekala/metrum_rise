//! Debug logging for Metrum Rise.
//!
//! Controlled at runtime via environment variables:
//! - `METRUM_DEBUG=1` — general debug logging (`./run.sh --debug`)
//! - `METRUM_DEBUG_FILTER=world-editor` — world-editor scoped logging
//!   (`./run.sh --debug world-editor` or `./run.sh --debug-world-editor`)
//! - `METRUM_DEBUG_TRAFFIC=1` — traffic/routing debug (`./run.sh --debug traffic`)
//! - `METRUM_DEBUG_SIM=1` — hourly simulation summaries (`./run.sh --debug-sim`)
//! - `METRUM_DEBUG_PERF=1` — renderer and simulation frame timing summaries
//! - `METRUM_DEBUG_FILTER=economy,border,...` — optional category filter for general debug logs
//! - `METRUM_CRASH_DIAGNOSTICS=1` — release-safe panic/hang dumps and flight-recorder capture
//! - `METRUM_HANG_WATCHDOG_MS=10000` — override the crash-diagnostics hang timeout; `0` disables it
//! - `METRUM_HANG_ABORT=1` — abort after writing the first hang dump
//!
//! Output goes to stdout so it appears in the terminal alongside Godot's output.
//! Use [`debug_log!`] and [`traffic_log!`] throughout the codebase — both are
//! no-ops when the respective flag is off, with only an atomic bool check overhead.

mod crash;

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

pub use crash::{CRASH_DIAGNOSTICS_ENABLED, is_crash_diagnostics_enabled};
pub(crate) use crash::{
    CrashCommand, CrashSimSnapshot, flush_crash_diagnostics, record_crash_command,
    record_crash_frame, record_crash_phase, suspend_hang_watchdog,
};

/// General debug flag — set once at startup by [`init`], read by [`debug_log!`].
pub static ENABLED: AtomicBool = AtomicBool::new(false);

/// Traffic/routing debug flag — set by `METRUM_DEBUG_TRAFFIC=1` / `./run.sh --debug traffic`.
pub static TRAFFIC_ENABLED: AtomicBool = AtomicBool::new(false);
/// Simulation-summary debug flag — set by `METRUM_DEBUG_SIM=1` / `./run.sh --debug-sim`.
pub static SIM_ENABLED: AtomicBool = AtomicBool::new(false);
/// Performance debug flag — set by `METRUM_DEBUG_PERF=1`.
pub static PERF_ENABLED: AtomicBool = AtomicBool::new(false);

/// Optional comma-separated category filter for general debug logs.
static FILTER: OnceLock<Vec<String>> = OnceLock::new();

/// Reads `METRUM_DEBUG`, `METRUM_DEBUG_TRAFFIC`, and `METRUM_DEBUG_SIM` from the
/// environment and arms
/// the global flags. Call once from the GDExtension init hook; safe to call multiple times.
pub fn init() {
    let on = std::env::var("METRUM_DEBUG")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false);
    ENABLED.store(on, Ordering::Relaxed);
    let filter_value = std::env::var("METRUM_DEBUG_FILTER").ok();
    let parsed_filter = filter_value
        .as_deref()
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(|entry| entry.to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let _ = FILTER.set(parsed_filter);
    if on {
        match filter_value.as_deref() {
            Some(raw) if !raw.trim().is_empty() => println!(
                "[DEBUG] Metrum Rise debug logging enabled (METRUM_DEBUG=1, filter={})",
                raw
            ),
            _ => println!("[DEBUG] Metrum Rise debug logging enabled (METRUM_DEBUG=1)"),
        }
    }

    let traffic_on = std::env::var("METRUM_DEBUG_TRAFFIC")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false);
    TRAFFIC_ENABLED.store(traffic_on, Ordering::Relaxed);
    if traffic_on {
        println!("[DEBUG] Traffic/routing debug logging enabled (METRUM_DEBUG_TRAFFIC=1)");
    }

    let sim_on = std::env::var("METRUM_DEBUG_SIM")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false);
    SIM_ENABLED.store(sim_on, Ordering::Relaxed);
    if sim_on {
        println!("[DEBUG] Simulation console debug enabled (METRUM_DEBUG_SIM=1)");
    }

    let perf_on = std::env::var("METRUM_DEBUG_PERF")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false);
    PERF_ENABLED.store(perf_on, Ordering::Relaxed);
    if perf_on {
        println!("[DEBUG] Performance debug logging enabled (METRUM_DEBUG_PERF=1)");
    }

    crash::init();
}

/// Returns `true` if general debug logging is currently enabled.
#[inline(always)]
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Returns `true` if the given general-debug category should currently be emitted.
#[inline(always)]
pub fn category_enabled(category: &str) -> bool {
    if !is_enabled() {
        return false;
    }
    let Some(filter) = FILTER.get() else {
        return true;
    };
    if filter.is_empty() {
        return true;
    }
    let category = category.to_ascii_lowercase();
    filter.iter().any(|entry| entry == &category)
}

/// Returns `true` if traffic/routing debug logging is currently enabled.
#[inline(always)]
pub fn is_traffic_enabled() -> bool {
    TRAFFIC_ENABLED.load(Ordering::Relaxed)
}

/// Returns `true` if hourly simulation summaries should currently be emitted.
#[inline(always)]
pub fn is_sim_enabled() -> bool {
    SIM_ENABLED.load(Ordering::Relaxed)
}

/// Returns `true` if per-frame performance diagnostics should be emitted.
#[inline(always)]
pub fn is_perf_enabled() -> bool {
    PERF_ENABLED.load(Ordering::Relaxed)
}

/// Logs a categorised debug message to stdout when general debug mode is on.
///
/// Usage: `debug_log!("road", "phase={} duration={}µs", phase_name, elapsed);`
///
/// The first argument is a short category tag (e.g. `"road"`, `"agent"`, `"zone"`).
/// Output format: `[DEBUG:road] message`
#[macro_export]
macro_rules! debug_log {
    ($category:expr, $($arg:tt)*) => {
        if $crate::debug::category_enabled($category) {
            println!("[DEBUG:{}] {}", $category, format!($($arg)*));
        }
    };
}

/// Logs a traffic/routing debug message to stderr when `--debug traffic` is active.
///
/// Usage: `traffic_log!("[CCH_QUERY] find_path {}→{}: nodes={:?}", start, end, nodes);`
///
/// Output goes to stderr to match the existing eprintln! convention for these messages.
#[macro_export]
macro_rules! traffic_log {
    ($($arg:tt)*) => {
        if $crate::debug::is_traffic_enabled() {
            eprintln!($($arg)*);
        }
    };
}
