// SPDX-License-Identifier: GPL-2.0-only

//! Fixed debug channel registry and its runtime on/off mask.
//!
//! Every channel is one bit in a single [`AtomicU32`]. Checking a channel is one relaxed load
//! and a constant mask test, so disabled log calls never format or allocate. Environment
//! variables pick the startup mask once in [`crate::debug::init`]; the console flips bits later.

use std::sync::atomic::{AtomicU32, Ordering};

/// When a channel is switched on by the environment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Group {
    // On with a bare `METRUM_DEBUG=1`, or when named in `METRUM_DEBUG_FILTER`.
    General,
    // Off with a bare `METRUM_DEBUG=1`; needs its own variable or a filter entry.
    OptIn,
    // Always on and buffer-only: the source already printed the message.
    Always,
}

macro_rules! channels {
    ($( $(#[$doc:meta])* $variant:ident = $tag:literal, $group:ident; )*) => {
        /// One debug log channel. The tag is the exact text inside `[DEBUG:<tag>]`.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(u8)]
        pub enum Channel {
            $( $(#[$doc])* $variant, )*
        }

        impl Channel {
            /// Every channel in registry order; the console lists them in this order.
            pub const ALL: &'static [Channel] = &[$(Channel::$variant),*];

            /// The stable channel name printed as `[DEBUG:<tag>]` and used by filters.
            pub const fn tag(self) -> &'static str {
                match self {
                    $(Channel::$variant => $tag,)*
                }
            }

            const fn group(self) -> Group {
                match self {
                    $(Channel::$variant => Group::$group,)*
                }
            }
        }
    };
}

channels! {
    /// Road placement, preview, commit and road-surface geometry.
    Road = "road", General;
    /// Traffic and routing: junctions, lane changes, replans and network connectivity.
    Traffic = "traffic", OptIn;
    /// Economy hour and day summaries.
    Economy = "economy", General;
    /// Per-phase economy timing detail.
    EconomyPhases = "economy_phases", General;
    /// Workplace assignment timing detail.
    EconomyWorkplace = "economy_workplace", General;
    /// Household admission and building spawn.
    Spawn = "spawn", General;
    /// Hourly simulation summaries.
    Sim = "sim", OptIn;
    /// Frame and preview CPU timing.
    Perf = "perf", OptIn;
    /// Terrain and water patch residency, meshing and timing.
    Terrain = "terrain", OptIn;
    /// Building-site mesh, material and grading samples.
    Buildings = "buildings", OptIn;
    /// Save and load.
    Save = "save", General;
    /// World editor activity.
    WorldEditor = "world-editor", General;
    /// Asset editor export and validation.
    AssetEditor = "asset-editor", General;
    /// Road intersection candidate timing.
    Isect = "isect", General;
    /// Simulation node start-up.
    Init = "init", General;
    /// Scene lighting and shadow-policy summaries (`--debug terrain-visual lighting`).
    Lighting = "lighting", OptIn;
    /// Warnings, errors and rejections the game already reports, kept for later inspection.
    Events = "events", Always;
}

const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

impl Channel {
    /// Looks up a channel by its exact tag. O(channels).
    pub const fn from_name(name: &str) -> Option<Channel> {
        let mut i = 0;
        while i < Self::ALL.len() {
            if str_eq(Self::ALL[i].tag(), name) {
                return Some(Self::ALL[i]);
            }
            i += 1;
        }
        None
    }

    /// Compile-time lookup used by `debug_log!("tag", ...)`; an unknown tag fails the build.
    pub const fn from_tag(tag: &str) -> Channel {
        match Self::from_name(tag) {
            Some(channel) => channel,
            None => panic!("unknown debug channel tag"),
        }
    }

    /// This channel's bit in the shared mask.
    pub const fn bit(self) -> u32 {
        1 << self as u8
    }

    /// Returns `true` while the channel is on. One relaxed atomic load; never allocates.
    #[inline(always)]
    pub fn enabled(self) -> bool {
        MASK.load(Ordering::Relaxed) & self.bit() != 0
    }

    /// Switches the channel at runtime. `events` cannot be switched off.
    pub fn set_enabled(self, on: bool) {
        if on {
            MASK.fetch_or(self.bit(), Ordering::Relaxed);
        } else if !self.locked() {
            MASK.fetch_and(!self.bit(), Ordering::Relaxed);
        }
    }

    /// Returns `true` for channels that stay on regardless of the console.
    pub const fn locked(self) -> bool {
        matches!(self.group(), Group::Always)
    }

    /// Returns `true` when enabled lines are echoed to stdout as `[DEBUG:<tag>] msg`.
    /// `events` is buffer-only because its sources already print the original message.
    pub const fn prints_to_stdout(self) -> bool {
        !matches!(self.group(), Group::Always)
    }
}

const fn group_mask(group: Group) -> u32 {
    let mut mask = 0;
    let mut i = 0;
    while i < Channel::ALL.len() {
        if matches!(
            (Channel::ALL[i].group(), group),
            (Group::General, Group::General)
                | (Group::OptIn, Group::OptIn)
                | (Group::Always, Group::Always)
        ) {
            mask |= Channel::ALL[i].bit();
        }
        i += 1;
    }
    mask
}

/// Channels a bare `METRUM_DEBUG=1` turns on.
const GENERAL_MASK: u32 = group_mask(Group::General);
/// Channels that can never be switched off.
const ALWAYS_MASK: u32 = group_mask(Group::Always);

static MASK: AtomicU32 = AtomicU32::new(ALWAYS_MASK);

/// Returns the full channel mask, one bit per [`Channel`].
pub fn mask() -> u32 {
    MASK.load(Ordering::Relaxed)
}

/// Replaces the full channel mask; locked channels stay on.
pub fn set_mask(mask: u32) {
    MASK.store(mask | ALWAYS_MASK, Ordering::Relaxed);
}

/// Filter entries that name a startup mode of a channel rather than the channel itself.
const FILTER_ALIASES: &[(&str, Channel)] = &[
    ("terrain-verbose", Channel::Terrain),
    ("terrain-full", Channel::Terrain),
    ("terrain-lod1", Channel::Terrain),
    ("terrain-full-lod1", Channel::Terrain),
    ("building-sites", Channel::Buildings),
];

/// Returns `true` for a set boolean environment value: non-empty and not `0`.
pub(crate) fn env_flag(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.is_empty() && value != "0")
}

/// Overlay switch: a non-empty variable decides (`0` is off); otherwise any `markers` substring
/// of the lowercased raw filter turns it on.
pub(crate) fn overlay_flag(value: Option<&str>, raw_filter: &str, markers: &[&str]) -> bool {
    match value.map(str::trim) {
        Some(value) if !value.is_empty() => value != "0",
        _ => markers.iter().any(|marker| raw_filter.contains(marker)),
    }
}

/// Splits `METRUM_DEBUG_FILTER` into trimmed, lowercased entries.
pub(crate) fn filter_entries(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

/// Channels `METRUM_DEBUG=1` selects: the filtered channels, or every general channel.
fn debug_mask(filter: &[String]) -> u32 {
    if filter.is_empty() {
        return GENERAL_MASK;
    }
    filter.iter().fold(0, |mask, entry| {
        let channel = Channel::from_name(entry).or_else(|| {
            FILTER_ALIASES
                .iter()
                .find(|(alias, _)| alias == entry)
                .map(|&(_, channel)| channel)
        });
        mask | channel.map_or(0, Channel::bit)
    })
}

/// Startup channel state derived from the environment by [`crate::debug::init`].
pub(crate) struct StartupChannels {
    /// Initial mask, including locked channels.
    pub mask: u32,
    /// Channels `METRUM_DEBUG=1` selects; the asset editor turns these on.
    pub debug_mask: u32,
}

/// Derives the startup mask from `METRUM_DEBUG*` values; `env` returns a variable's value.
pub(crate) fn startup_channels(env: impl Fn(&str) -> Option<String>) -> StartupChannels {
    let flag = |name: &str| env_flag(env(name).as_deref());
    let filter = filter_entries(env("METRUM_DEBUG_FILTER").as_deref());
    let debug_mask = debug_mask(&filter);
    let mut mask = ALWAYS_MASK;
    if flag("METRUM_DEBUG") {
        mask |= debug_mask;
    }
    for (name, channel) in [
        ("METRUM_DEBUG_TRAFFIC", Channel::Traffic),
        ("METRUM_DEBUG_SIM", Channel::Sim),
        ("METRUM_DEBUG_PERF", Channel::Perf),
        ("METRUM_DEBUG_TERRAIN", Channel::Terrain),
        ("METRUM_DEBUG_BUILDINGS", Channel::Buildings),
    ] {
        if flag(name) {
            mask |= channel.bit();
        }
    }
    // The road-surface probe exists to print its dumps on the road channel.
    let raw_filter = env("METRUM_DEBUG_FILTER")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if overlay_flag(
        env("METRUM_DEBUG_ROAD_PROBE").as_deref(),
        &raw_filter,
        &["road-probe"],
    ) {
        mask |= Channel::Road.bit();
    }
    let visual = env("METRUM_DEBUG_TERRAIN_VISUAL").unwrap_or_default();
    if matches!(
        visual.trim().to_ascii_lowercase().as_str(),
        "lighting" | "light" | "sun"
    ) {
        mask |= Channel::Lighting.bit();
    }
    StartupChannels { mask, debug_mask }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn startup(vars: &[(&str, &str)]) -> u32 {
        startup_channels(|name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        })
        .mask
    }

    fn bits(channels: &[Channel]) -> u32 {
        channels
            .iter()
            .fold(0, |mask, channel| mask | channel.bit())
    }

    #[test]
    fn tags_round_trip_and_fit_the_mask() {
        assert!(Channel::ALL.len() <= 32);
        for &channel in Channel::ALL {
            assert_eq!(Channel::from_name(channel.tag()), Some(channel));
        }
        assert_eq!(Channel::from_name("Road"), None);
        assert_eq!(
            const { Channel::from_tag("world-editor") },
            Channel::WorldEditor
        );
    }

    #[test]
    fn empty_environment_keeps_only_events() {
        assert_eq!(startup(&[]), Channel::Events.bit());
        assert_eq!(startup(&[("METRUM_DEBUG", "0")]), Channel::Events.bit());
    }

    #[test]
    fn bare_debug_enables_general_channels_only() {
        let mask = startup(&[("METRUM_DEBUG", "1")]);
        for channel in [
            Channel::Road,
            Channel::Economy,
            Channel::EconomyPhases,
            Channel::Spawn,
        ] {
            assert!(mask & channel.bit() != 0, "{channel:?}");
        }
        for channel in [
            Channel::Traffic,
            Channel::Sim,
            Channel::Perf,
            Channel::Terrain,
        ] {
            assert!(mask & channel.bit() == 0, "{channel:?}");
        }
    }

    #[test]
    fn filter_selects_named_channels_and_aliases() {
        let mask = startup(&[
            ("METRUM_DEBUG", "1"),
            (
                "METRUM_DEBUG_FILTER",
                " Road, terrain-full-lod1,border ,perf",
            ),
        ]);
        assert_eq!(
            mask,
            bits(&[
                Channel::Road,
                Channel::Terrain,
                Channel::Perf,
                Channel::Events
            ])
        );
        // A filter alone selects nothing; run.sh sets METRUM_DEBUG with it.
        assert_eq!(
            startup(&[("METRUM_DEBUG_FILTER", "road")]),
            Channel::Events.bit()
        );
    }

    #[test]
    fn dedicated_variables_enable_opt_in_channels() {
        let mask = startup(&[
            ("METRUM_DEBUG_TRAFFIC", "1"),
            ("METRUM_DEBUG_SIM", "yes"),
            ("METRUM_DEBUG_PERF", "1"),
            ("METRUM_DEBUG_TERRAIN", "1"),
            ("METRUM_DEBUG_BUILDINGS", "0"),
            ("METRUM_DEBUG_TERRAIN_VISUAL", " Sun"),
        ]);
        assert_eq!(
            mask,
            bits(&[
                Channel::Traffic,
                Channel::Sim,
                Channel::Perf,
                Channel::Terrain,
                Channel::Lighting,
                Channel::Events
            ])
        );
    }

    #[test]
    fn runtime_toggle_flips_one_bit_and_events_stay_on() {
        // Isect is the least used channel, so parallel tests never depend on its state.
        let before = mask();
        Channel::Isect.set_enabled(true);
        assert!(Channel::Isect.enabled());
        Channel::Isect.set_enabled(false);
        assert!(!Channel::Isect.enabled());
        Channel::Events.set_enabled(false);
        assert!(Channel::Events.enabled());
        assert_eq!(
            mask() & !Channel::Isect.bit(),
            before & !Channel::Isect.bit()
        );
    }
}
