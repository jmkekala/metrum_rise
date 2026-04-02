//! Debug logging for Metrum Rise.
//!
//! Controlled at runtime via the `METRUM_DEBUG` environment variable.
//! Set it before launching: `METRUM_DEBUG=1 godot …` or use `./run.sh --debug`.
//!
//! Output goes to stdout so it appears in the terminal alongside Godot's output.
//! Use the [`debug_log!`] macro throughout the codebase — it is a no-op when
//! debug mode is off, with only an atomic bool check as overhead.

use std::sync::atomic::{AtomicBool, Ordering};

/// Global flag — set once at startup by [`init`], read by [`debug_log!`].
pub static ENABLED: AtomicBool = AtomicBool::new(false);

/// Reads `METRUM_DEBUG` from the environment and arms the global flag.
///
/// Call this once from the GDExtension init hook. Safe to call multiple times.
pub fn init() {
    let on = std::env::var("METRUM_DEBUG")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false);
    ENABLED.store(on, Ordering::Relaxed);
    if on {
        println!("[DEBUG] Metrum Rise debug logging enabled (METRUM_DEBUG=1)");
    }
}

/// Returns `true` if debug logging is currently enabled.
#[inline(always)]
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Logs a categorised debug message to stdout when debug mode is on.
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
