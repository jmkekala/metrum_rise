// SPDX-License-Identifier: GPL-2.0-only

//! Godot bridge to the shared debug channels and log ring.
//!
//! Stateless and independent of `SimulationNode`, so the game, the world editor and the asset
//! editor share one channel state and one ordered log. Every method is static and thread-safe;
//! GDScript calls them as `MetrumDebug.method(...)`.

use crate::debug::{self, Channel, Source, log, visual};
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

    /// Writes channel state, visual and startup modes and the buffered log to `path` (a `user://` or
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

    /// Startup-only debug switches as `name -> bool`, in a fixed order. These come from
    /// `METRUM_DEBUG_*` variables at launch and do not change at runtime.
    #[func]
    pub fn startup_modes() -> VarDictionary {
        let mut modes = VarDictionary::new();
        for (name, on) in debug::startup_modes() {
            modes.set(*name, *on);
        }
        modes
    }

    /// One startup-only debug switch. Unknown names are `false`.
    #[func]
    pub fn startup_mode(name: GString) -> bool {
        debug::startup_mode(&name.to_string())
    }

    /// Every visual debug mode name in console order. Unlike startup modes, these switch at
    /// runtime; launch variables only set the starting value.
    #[func]
    pub fn visual_mode_names() -> PackedStringArray {
        visual::MODES
            .iter()
            .map(|mode| GString::from(mode.name))
            .collect()
    }

    /// The value names a visual mode accepts, in index order with `off` first. Empty for
    /// unknown modes.
    #[func]
    pub fn visual_mode_values(name: GString) -> PackedStringArray {
        visual::mode(&name.to_string()).map_or_else(PackedStringArray::new, |mode| {
            mode.values
                .iter()
                .map(|value| GString::from(*value))
                .collect()
        })
    }

    /// The selected value name of a visual mode; unknown modes are `off`.
    #[func]
    pub fn visual_mode(name: GString) -> GString {
        visual::mode(&name.to_string())
            .map_or("off", |mode| mode.value_name(mode.index()))
            .into()
    }

    /// The selected value index of a visual mode, which is also the shader's mode number;
    /// `0` is off and unknown modes are `0`.
    #[func]
    pub fn visual_mode_index(name: GString) -> i64 {
        visual::mode(&name.to_string()).map_or(0, |mode| i64::from(mode.index()))
    }

    /// Switches a visual mode by value name, alias or index. Returns `false`, with the
    /// reason logged on `events`, for an unknown mode or value. Renderers apply the change
    /// through `DebugConsole.set_visual_mode`, which calls this.
    #[func]
    pub fn set_visual_mode(name: GString, value: GString) -> bool {
        visual::set(&name.to_string(), &value.to_string()).is_ok()
    }

    /// Returns `true` when the game was launched with `METRUM_DEBUG=1`.
    #[func]
    pub fn is_debug_session() -> bool {
        debug::debug_session()
    }
}
