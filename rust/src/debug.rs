// SPDX-License-Identifier: GPL-2.0-only

//! Debug logging for Metrum Rise: one fixed set of channels shared by Rust and GDScript.
//!
//! [`Channel`] lists every channel; each is one bit in a single atomic mask, switchable at
//! runtime from the in-game console through the Godot bridge (`MetrumDebug`). Enabled lines
//! print to stdout as `[DEBUG:<tag>] message` and are kept in a bounded ring ([`log`]) that
//! the console reads and dumps. `events` is always on and buffer-only.
//!
//! The environment sets the startup state:
//! - `METRUM_DEBUG=1` — every general channel (`./run.sh --debug`)
//! - `METRUM_DEBUG_FILTER=road,economy,...` — with `METRUM_DEBUG=1`, only the named channels
//!   (`./run.sh --debug <category>`); terrain and building-site aliases are accepted
//! - `METRUM_DEBUG_TRAFFIC=1` — `traffic` (`./run.sh --debug traffic`)
//! - `METRUM_DEBUG_SIM=1` — `sim`, hourly simulation summaries (`./run.sh --debug-sim`)
//! - `METRUM_DEBUG_PERF=1` — `perf`, renderer and simulation frame timing summaries
//! - `METRUM_DEBUG_TERRAIN=1` / `METRUM_DEBUG_BUILDINGS=1` — `terrain` / `buildings`
//! - `METRUM_CRASH_DIAGNOSTICS=1` — release-safe panic/hang dumps and flight-recorder capture
//! - `METRUM_HANG_WATCHDOG_MS=10000` — override the crash-diagnostics hang timeout; `0` disables it
//! - `METRUM_HANG_ABORT=1` — abort after writing the first hang dump
//!
//! Other `METRUM_DEBUG_*` variables select startup-only modes ([`startup_modes`]) or the
//! starting value of a runtime-switchable visual mode ([`visual`]).
//! Use [`debug_log!`](crate::debug_log!) and [`traffic_log!`](crate::traffic_log!)
//! throughout the codebase. A disabled call is one relaxed atomic load and never allocates.

mod crash;
pub(crate) mod money;
pub(crate) mod work;

pub mod channel;
pub mod log;
pub mod visual;

use std::sync::OnceLock;

pub use channel::{Channel, mask, set_mask};
pub use crash::{CRASH_DIAGNOSTICS_ENABLED, is_crash_diagnostics_enabled};
pub(crate) use crash::{
    CrashCommand, CrashRoadGeometry, CrashSimSnapshot, flush_crash_diagnostics,
    record_crash_command, record_crash_frame, record_crash_phase, shutdown, suspend_hang_watchdog,
};
pub use log::{Source, emit};

struct Startup {
    debug_session: bool,
    debug_mask: u32,
    modes: Vec<(&'static str, bool)>,
}

static STARTUP: OnceLock<Startup> = OnceLock::new();

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// Derives the startup-only modes from the environment. These are read by renderers once at
/// load and are not switchable at runtime; visual modes live in [`visual`].
fn startup_modes_from(env: impl Fn(&str) -> Option<String>) -> Vec<(&'static str, bool)> {
    let flag = |name: &str| channel::env_flag(env(name).as_deref());
    let text = |name: &str| env(name).unwrap_or_default().trim().to_ascii_lowercase();
    let raw_filter = text("METRUM_DEBUG_FILTER");
    let filter = channel::filter_entries(Some(&raw_filter));
    let listed = |names: &[&str]| filter.iter().any(|entry| names.contains(&entry.as_str()));
    let overlay = |name: &str, markers: &[&str]| {
        channel::overlay_flag(env(name).as_deref(), &raw_filter, markers)
    };
    let dump = text("METRUM_DEBUG_BUILDING_SITES_DUMP");
    vec![
        (
            "road_geometry_dump",
            flag("METRUM_DEBUG_ROAD_GEOMETRY_DUMP"),
        ),
        (
            "road_surface",
            overlay("METRUM_DEBUG_SURFACE", &["road-surface", "surface-debug"]),
        ),
        (
            "road_probe",
            overlay("METRUM_DEBUG_ROAD_PROBE", &["road-probe"]),
        ),
        (
            "terrain_verbose",
            flag("METRUM_DEBUG_TERRAIN_VERBOSE") || listed(&["terrain-verbose"]),
        ),
        (
            "building_sites_dump",
            matches!(dump.as_str(), "1" | "true" | "yes" | "full"),
        ),
    ]
}

/// Reads the `METRUM_DEBUG*` environment, sets the startup channel mask and visual modes and
/// captures the startup-only modes. Call once from the GDExtension init hook; repeated calls
/// re-apply the environment's channel mask and visual modes.
pub fn init() {
    log::start_clock();
    let channels = channel::startup_channels(env);
    set_mask(channels.mask);
    visual::init(env);
    let debug_session = channel::env_flag(env("METRUM_DEBUG").as_deref());
    STARTUP.get_or_init(|| Startup {
        debug_session,
        debug_mask: channels.debug_mask,
        modes: startup_modes_from(env),
    });

    if debug_session {
        match env("METRUM_DEBUG_FILTER").as_deref() {
            Some(raw) if !raw.trim().is_empty() => println!(
                "[DEBUG] Metrum Rise debug logging enabled (METRUM_DEBUG=1, filter={})",
                raw
            ),
            _ => println!("[DEBUG] Metrum Rise debug logging enabled (METRUM_DEBUG=1)"),
        }
    }
    if channel::env_flag(env("METRUM_DEBUG_TRAFFIC").as_deref()) {
        println!("[DEBUG] Traffic/routing debug logging enabled (METRUM_DEBUG_TRAFFIC=1)");
    }
    if channel::env_flag(env("METRUM_DEBUG_SIM").as_deref()) {
        println!("[DEBUG] Simulation console debug enabled (METRUM_DEBUG_SIM=1)");
    }
    if channel::env_flag(env("METRUM_DEBUG_PERF").as_deref()) {
        println!("[DEBUG] Performance debug logging enabled (METRUM_DEBUG_PERF=1)");
    }

    crash::init();
}

/// Returns `true` when the game was started with `METRUM_DEBUG=1` (`./run.sh --debug ...`).
/// This is the launch-time debug session, not the state of any channel.
pub fn debug_session() -> bool {
    STARTUP.get().is_some_and(|startup| startup.debug_session)
}

/// Turns on the channels `METRUM_DEBUG=1` would select: the `METRUM_DEBUG_FILTER` channels,
/// or every general channel without a filter. The asset editor always logs these.
pub fn enable_debug_channels() {
    let debug_mask = STARTUP.get().map_or_else(
        || channel::startup_channels(|_| None).debug_mask,
        |startup| startup.debug_mask,
    );
    set_mask(mask() | debug_mask);
}

/// Startup-only debug switches in a fixed order, as `(name, on)`. Empty before [`init`].
pub fn startup_modes() -> &'static [(&'static str, bool)] {
    STARTUP.get().map_or(&[], |startup| &startup.modes)
}

/// Returns one startup-only debug switch by name; unknown names are off.
pub fn startup_mode(name: &str) -> bool {
    startup_modes().iter().any(|&(mode, on)| mode == name && on)
}

/// Logs a debug line on a channel when that channel is on.
///
/// Usage: `debug_log!("road", "phase={} duration={}µs", phase_name, elapsed);`
///
/// The first argument is a channel tag literal, checked at compile time, or a [`Channel`]
/// value. Enabled lines print to stdout as `[DEBUG:road] message` and go into the console
/// log. A disabled call is one relaxed atomic load: arguments are neither evaluated nor
/// formatted, so it never allocates.
#[macro_export]
macro_rules! debug_log {
    ($channel:literal, $($arg:tt)*) => {
        $crate::debug_log!(const { $crate::debug::Channel::from_tag($channel) }, $($arg)*)
    };
    ($channel:expr, $($arg:tt)*) => {{
        let channel: $crate::debug::Channel = $channel;
        if channel.enabled() {
            $crate::debug::emit(channel, ::std::format_args!($($arg)*));
        }
    }};
}

/// Logs a traffic/routing line on the `traffic` channel (`--debug traffic`).
///
/// Usage: `traffic_log!("[CCH_QUERY] find_path {}→{}: nodes={:?}", start, end, nodes);`
///
/// Output goes to stdout as `[DEBUG:traffic] message`; disabled calls cost one atomic load.
#[macro_export]
macro_rules! traffic_log {
    ($($arg:tt)*) => {
        $crate::debug_log!($crate::debug::Channel::Traffic, $($arg)*)
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn disabled_channel_does_not_evaluate_arguments() {
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        fn counted() -> usize {
            CALLS.fetch_add(1, Ordering::Relaxed)
        }
        // Only this test toggles Init, so the state is not shared with parallel tests.
        Channel::Init.set_enabled(false);
        crate::debug_log!("init", "{}", counted());
        crate::debug_log!(Channel::Init, "{}", counted());
        assert_eq!(CALLS.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn startup_modes_follow_variables_and_filter_entries() {
        let vars = [
            ("METRUM_DEBUG_FILTER", "road,terrain-verbose,road-probe"),
            ("METRUM_DEBUG_SURFACE", "0"),
            ("METRUM_DEBUG_BUILDING_SITES_DUMP", "full"),
        ];
        let modes = startup_modes_from(|name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        });
        let get = |name: &str| modes.iter().find(|(mode, _)| *mode == name).unwrap().1;
        assert!(!get("road_surface"));
        assert!(get("road_probe"));
        assert!(get("terrain_verbose"));
        assert!(!get("road_geometry_dump"));
        assert!(get("building_sites_dump"));
    }
}
