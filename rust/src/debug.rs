//! Debug logging for Metrum Rise.
//!
//! Controlled at runtime via environment variables:
//! - `METRUM_DEBUG=1` — general debug logging (`./run.sh --debug`)
//! - `METRUM_DEBUG_TRAFFIC=1` — traffic/routing debug (`./run.sh --debug traffic`)
//!
//! Output goes to stdout so it appears in the terminal alongside Godot's output.
//! Use [`debug_log!`] and [`traffic_log!`] throughout the codebase — both are
//! no-ops when the respective flag is off, with only an atomic bool check overhead.

use std::sync::atomic::{AtomicBool, Ordering};

/// General debug flag — set once at startup by [`init`], read by [`debug_log!`].
pub static ENABLED: AtomicBool = AtomicBool::new(false);

/// Traffic/routing debug flag — set by `METRUM_DEBUG_TRAFFIC=1` / `./run.sh --debug traffic`.
pub static TRAFFIC_ENABLED: AtomicBool = AtomicBool::new(false);

/// Reads `METRUM_DEBUG` and `METRUM_DEBUG_TRAFFIC` from the environment and arms
/// the global flags. Call once from the GDExtension init hook; safe to call multiple times.
pub fn init() {
    let on = std::env::var("METRUM_DEBUG")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false);
    ENABLED.store(on, Ordering::Relaxed);
    if on {
        println!("[DEBUG] Metrum Rise debug logging enabled (METRUM_DEBUG=1)");
    }

    let traffic_on = std::env::var("METRUM_DEBUG_TRAFFIC")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false);
    TRAFFIC_ENABLED.store(traffic_on, Ordering::Relaxed);
    if traffic_on {
        println!("[DEBUG] Traffic/routing debug logging enabled (METRUM_DEBUG_TRAFFIC=1)");
    }
}

/// Returns `true` if general debug logging is currently enabled.
#[inline(always)]
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Returns `true` if traffic/routing debug logging is currently enabled.
#[inline(always)]
pub fn is_traffic_enabled() -> bool {
    TRAFFIC_ENABLED.load(Ordering::Relaxed)
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
        if $crate::debug::is_enabled() {
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
