// SPDX-License-Identifier: GPL-2.0-only

//! Godot bridge to the shared debug channels and log ring.
//!
//! Stateless and independent of `SimulationNode`, so the game, the world editor and the asset
//! editor share one channel state and one ordered log. Every method is static and thread-safe;
//! GDScript calls them as `MetrumDebug.method(...)`.

use crate::debug::{self, Channel, Source, StartupValue, log};
use godot::classes::ProjectSettings;
use godot::prelude::*;

/// Script access to debug channels, the debug log and dumps.
#[derive(GodotClass)]
#[class(init, base = RefCounted)]
pub struct MetrumDebug;

fn channel(name: &GString) -> Option<Channel> {
    Channel::from_name(&name.to_string())
}

#[godot_api]
impl MetrumDebug {
    /// Every channel tag in registry order.
    #[func]
    pub fn channel_names() -> PackedStringArray {
        Channel::ALL
            .iter()
            .map(|channel| GString::from(channel.tag()))
            .collect()
    }

    /// Returns `true` while the named channel is on; unknown names are off.
    #[func]
    pub fn is_enabled(channel_name: GString) -> bool {
        channel(&channel_name).is_some_and(Channel::enabled)
    }

    /// Switches a channel at runtime. Returns `false` for unknown names and for switching
    /// off a locked channel (`events`).
    #[func]
    pub fn set_enabled(channel_name: GString, on: bool) -> bool {
        let Some(channel) = channel(&channel_name) else {
            return false;
        };
        channel.set_enabled(on);
        channel.enabled() == on
    }

    /// Returns `true` for channels that cannot be switched off.
    #[func]
    pub fn is_locked(channel_name: GString) -> bool {
        channel(&channel_name).is_some_and(Channel::locked)
    }

    /// Logs a GDScript line on a channel. Prints `[DEBUG:<tag>] message` to stdout like Rust
    /// lines and joins the same ordered log. Returns `false` if the channel is off or unknown.
    #[func]
    pub fn log(channel_name: GString, message: GString) -> bool {
        channel(&channel_name)
            .is_some_and(|channel| log::push(channel, Source::Godot, message.to_string()))
    }

    /// Returns up to `max_lines` lines newer than `after_seq`, oldest first, as packed
    /// columns: `seq`, `time_ms`, `channel`, `source`, `message`, plus `last_seq`, the newest
    /// stored sequence number. Lines older than the ring's capacity are gone.
    #[func]
    pub fn read_since(after_seq: i64, max_lines: i64) -> VarDictionary {
        let mut seq = PackedInt64Array::new();
        let mut time_ms = PackedInt64Array::new();
        let mut channel = PackedStringArray::new();
        let mut source = PackedStringArray::new();
        let mut message = PackedStringArray::new();
        let last_seq =
            log::read_since(after_seq.max(0) as u64, max_lines.max(0) as usize, |line| {
                seq.push(line.seq as i64);
                time_ms.push(line.time_ms as i64);
                channel.push(line.channel.tag());
                source.push(line.source.label());
                message.push(line.message.as_str());
            });
        vdict! {
            "last_seq": last_seq as i64,
            "seq": seq,
            "time_ms": time_ms,
            "channel": channel,
            "source": source,
            "message": message,
        }
    }

    /// Drops every buffered line; sequence numbers keep counting up.
    #[func]
    pub fn clear() {
        log::clear();
    }

    /// Writes channel state, startup modes and the buffered log to `path` (a `user://` or
    /// absolute path), creating folders as needed. Returns the absolute path, or an empty
    /// string on failure after pushing a warning.
    #[func]
    pub fn write_dump(path: GString) -> GString {
        let absolute = ProjectSettings::singleton().globalize_path(&path);
        match log::write_dump(std::path::Path::new(&absolute.to_string())) {
            Ok(()) => absolute,
            Err(error) => {
                godot_warn!("Debug dump to {absolute} failed: {error}");
                GString::new()
            }
        }
    }

    /// Startup-only debug modes as `name -> bool | String`, in a fixed order. These come
    /// from `METRUM_DEBUG_*` variables at launch and do not change at runtime.
    #[func]
    pub fn startup_modes() -> VarDictionary {
        let mut modes = VarDictionary::new();
        for (name, value) in debug::startup_modes() {
            modes.set(*name, startup_variant(value));
        }
        modes
    }

    /// One startup-only debug mode: a bool switch or a lowercased mode name. Unknown names
    /// return `false`.
    #[func]
    pub fn startup_mode(name: GString) -> Variant {
        debug::startup_mode(&name.to_string()).map_or(false.to_variant(), startup_variant)
    }

    /// Returns `true` when the game was launched with `METRUM_DEBUG=1`.
    #[func]
    pub fn is_debug_session() -> bool {
        debug::debug_session()
    }
}

fn startup_variant(value: &StartupValue) -> Variant {
    match value {
        StartupValue::Flag(on) => on.to_variant(),
        StartupValue::Text(text) => text.to_variant(),
    }
}
