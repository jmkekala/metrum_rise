// SPDX-License-Identifier: GPL-2.0-only

//! Visual debug modes: renderer views the console switches at runtime.
//!
//! Each mode has a fixed list of value names; the selected value is its index, stored in one
//! [`AtomicU8`] per mode. Index `0` is always `off`, and for shader-driven modes the index is
//! the shader's mode number. Launch variables pick the starting value once in
//! [`crate::debug::init`]. Nothing on the simulation tick reads this state.

use super::channel::{env_flag, filter_entries};
use super::{Channel, Source, log};
use std::sync::atomic::{AtomicU8, Ordering};

/// One visual debug mode and the names it accepts.
pub struct VisualMode {
    /// Stable mode name used by the console and `MetrumDebug`.
    pub name: &'static str,
    /// The launch variable that sets the starting value.
    pub env: &'static str,
    /// Value names in index order; the first is always `off`.
    pub values: &'static [&'static str],
    // Extra spellings accepted for a value, as `(alias, value)`.
    aliases: &'static [(&'static str, &'static str)],
}

const FLAG_VALUES: &[&str] = &["off", "on"];
const FLAG_ALIASES: &[(&str, &str)] = &[("true", "on"), ("yes", "on")];

/// Every visual debug mode in console order.
pub const MODES: &[VisualMode] = &[
    VisualMode {
        name: "terrain_visual",
        env: "METRUM_DEBUG_TERRAIN_VISUAL",
        values: &[
            "off",
            "patch",
            "lod",
            "height",
            "relief",
            "shore",
            "water-depth",
            "water-lod",
            "water-patch",
            "water-material",
            "lighting",
            "graded-delta",
            "graded-sdf",
            "graded-level",
            "graded-slot",
        ],
        aliases: &[
            ("patches", "patch"),
            ("lods", "lod"),
            ("shoreline", "shore"),
            ("water", "water-depth"),
            ("depth", "water-depth"),
            ("water-mat", "water-material"),
            ("material-water", "water-material"),
            ("light", "lighting"),
            ("sun", "lighting"),
            ("delta", "graded-delta"),
            ("sdf", "graded-sdf"),
        ],
    },
    VisualMode {
        name: "terrain_grass",
        env: "METRUM_DEBUG_TERRAIN_GRASS",
        values: &[
            "off",
            "raw",
            "macro",
            "mid",
            "micro",
            "fades",
            "material",
            "height",
            "mask",
            "luminance",
            "footprint",
        ],
        aliases: &[
            ("albedo", "raw"),
            ("fade", "fades"),
            ("visibility", "fades"),
            ("composite", "material"),
            ("grass-mask", "mask"),
            ("luma", "luminance"),
            ("brightness", "luminance"),
            ("footprints", "footprint"),
        ],
    },
    VisualMode {
        name: "building_sites_visual",
        env: "METRUM_DEBUG_BUILDING_SITES_VISUAL",
        values: &["off", "material"],
        aliases: &[
            ("materials", "material"),
            ("source", "material"),
            ("sources", "material"),
        ],
    },
    VisualMode {
        name: "pedestrian_vat",
        env: "METRUM_DEBUG_PEDESTRIAN_VAT",
        values: &["off", "rest", "uv", "offsets"],
        aliases: &[("offset", "offsets")],
    },
    VisualMode {
        name: "terrain_force_lod1",
        env: "METRUM_DEBUG_TERRAIN_FORCE_LOD1",
        values: FLAG_VALUES,
        aliases: FLAG_ALIASES,
    },
    VisualMode {
        name: "terrain_force_full_world",
        env: "METRUM_DEBUG_TERRAIN_FORCE_FULL_WORLD",
        values: FLAG_VALUES,
        aliases: FLAG_ALIASES,
    },
    VisualMode {
        name: "junction_connectors",
        env: "METRUM_DEBUG_JUNCTION_CONNECTORS",
        values: &["off", "arcs", "speed", "occupancy", "lanes"],
        aliases: &[
            ("on", "arcs"),
            ("connectors", "arcs"),
            ("turn-speed", "speed"),
            ("cars", "occupancy"),
        ],
    },
];

static STATE: [AtomicU8; MODES.len()] = [const { AtomicU8::new(0) }; MODES.len()];

impl VisualMode {
    /// Parses a value name, alias or index. Empty, `0`, `off` and `false` are `off`; anything
    /// else not listed is `None`.
    pub fn parse(&self, text: &str) -> Option<u8> {
        let text = text.trim().to_ascii_lowercase();
        if matches!(text.as_str(), "" | "off" | "false") {
            return Some(0);
        }
        if let Ok(index) = text.parse::<usize>() {
            return (index < self.values.len()).then_some(index as u8);
        }
        let name = self
            .aliases
            .iter()
            .find(|(alias, _)| *alias == text)
            .map_or(text.as_str(), |(_, value)| value);
        self.values
            .iter()
            .position(|value| *value == name)
            .map(|index| index as u8)
    }

    /// The value name for an index.
    pub fn value_name(&self, index: u8) -> &'static str {
        self.values
            .get(usize::from(index))
            .copied()
            .unwrap_or("off")
    }

    // Names are unique; a linear search over a handful of modes only runs on console switches.
    fn slot(&self) -> &'static AtomicU8 {
        let index = MODES
            .iter()
            .position(|mode| mode.name == self.name)
            .expect("visual modes come from MODES");
        &STATE[index]
    }

    /// The selected value index; `0` is off.
    pub fn index(&self) -> u8 {
        self.slot().load(Ordering::Relaxed)
    }
}

/// Looks a mode up by name.
pub fn mode(name: &str) -> Option<&'static VisualMode> {
    MODES.iter().find(|mode| mode.name == name)
}

/// Why [`set`] refused a switch.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// No mode has this name.
    UnknownMode,
    /// The mode exists but does not accept this value.
    UnknownValue,
}

/// Switches a mode by value name, alias or index and returns the new index. A refusal is
/// logged on `events` with the accepted names.
pub fn set(name: &str, value: &str) -> Result<u8, Refusal> {
    let Some(mode) = mode(name) else {
        let names: Vec<_> = MODES.iter().map(|mode| mode.name).collect();
        log::push(
            Channel::Events,
            Source::Rust,
            format!(
                "visual mode '{name}' refused: unknown mode; modes are {}",
                names.join(", ")
            ),
        );
        return Err(Refusal::UnknownMode);
    };
    let Some(index) = mode.parse(value) else {
        log::push(
            Channel::Events,
            Source::Rust,
            format!(
                "visual mode {name}='{value}' refused: unknown value; values are {}",
                mode.values.join(", ")
            ),
        );
        return Err(Refusal::UnknownValue);
    };
    mode.slot().store(index, Ordering::Relaxed);
    Ok(index)
}

/// Starting index of every mode, in [`MODES`] order, from the launch variables and the
/// `--pedestrian-vat-debug` argument. Unknown names start off, except that any set building
/// site variable shows the only overlay there is.
fn starting_indices(
    env: impl Fn(&str) -> Option<String>,
    args: impl IntoIterator<Item = String>,
) -> [u8; MODES.len()] {
    let filter = filter_entries(env("METRUM_DEBUG_FILTER").as_deref());
    let listed = |names: &[&str]| filter.iter().any(|entry| names.contains(&entry.as_str()));
    let pedestrian_arg = pedestrian_vat_arg(args);
    let mut indices = [0; MODES.len()];
    for (index, mode) in MODES.iter().enumerate() {
        let text = match mode.name {
            "pedestrian_vat" => pedestrian_arg.clone().or_else(|| env(mode.env)),
            _ => env(mode.env),
        };
        indices[index] = match mode.name {
            "terrain_force_lod1" => u8::from(
                env_flag(text.as_deref()) || listed(&["terrain-lod1", "terrain-full-lod1"]),
            ),
            "terrain_force_full_world" => u8::from(
                env_flag(text.as_deref()) || listed(&["terrain-full", "terrain-full-lod1"]),
            ),
            "building_sites_visual" => mode.parse(text.as_deref().unwrap_or_default()).unwrap_or(1),
            _ => mode.parse(text.as_deref().unwrap_or_default()).unwrap_or(0),
        };
    }
    indices
}

// The command line wins over the variable: `--pedestrian-vat-debug=<mode>` or
// `--pedestrian-vat-debug <mode>`.
fn pedestrian_vat_arg(args: impl IntoIterator<Item = String>) -> Option<String> {
    let mut found = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let arg = arg.trim().to_ascii_lowercase();
        if let Some(value) = arg.strip_prefix("--pedestrian-vat-debug=") {
            found = Some(value.to_owned());
        } else if arg == "--pedestrian-vat-debug" {
            found = args.next();
        }
    }
    found
}

/// Applies the launch values. Called from [`crate::debug::init`].
pub(crate) fn init(env: impl Fn(&str) -> Option<String>) {
    for (slot, index) in STATE.iter().zip(starting_indices(env, std::env::args())) {
        slot.store(index, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(name: &str) -> &'static VisualMode {
        mode(name).unwrap()
    }

    fn start(vars: &[(&str, &str)], args: &[&str]) -> Vec<(&'static str, &'static str)> {
        let indices = starting_indices(
            |name| {
                vars.iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.to_string())
            },
            args.iter().map(|arg| arg.to_string()),
        );
        MODES
            .iter()
            .zip(indices)
            .map(|(mode, index)| (mode.name, mode.value_name(index)))
            .collect()
    }

    #[test]
    fn values_aliases_and_indices_parse() {
        let terrain = lookup("terrain_visual");
        assert_eq!(terrain.parse(" Water-Depth "), Some(6));
        assert_eq!(terrain.parse("depth"), Some(6));
        assert_eq!(terrain.parse("sun"), Some(10));
        assert_eq!(terrain.parse("4"), Some(4));
        assert_eq!(terrain.parse("11"), None);
        assert_eq!(terrain.parse("false"), Some(0));
        assert_eq!(terrain.parse("bogus"), None);
        assert_eq!(lookup("terrain_grass").parse("luma"), Some(9));
        assert_eq!(lookup("terrain_grass").parse("footprint"), Some(10));
        assert_eq!(lookup("pedestrian_vat").parse("offset"), Some(3));
        assert_eq!(lookup("pedestrian_vat").parse("off"), Some(0));
        assert_eq!(lookup("building_sites_visual").parse("sources"), Some(1));
        assert_eq!(lookup("terrain_force_lod1").parse("on"), Some(1));
        assert_eq!(lookup("terrain_force_lod1").parse("yes"), Some(1));
        assert_eq!(lookup("terrain_force_lod1").parse("2"), None);
        assert_eq!(lookup("junction_connectors").parse("on"), Some(1));
        assert_eq!(lookup("junction_connectors").parse("lanes"), Some(4));
        for mode in MODES {
            assert_eq!(mode.values[0], "off");
            assert_eq!(mode.value_name(mode.values.len() as u8), "off");
            for (index, value) in mode.values.iter().enumerate() {
                assert_eq!(
                    mode.parse(value),
                    Some(index as u8),
                    "{}={value}",
                    mode.name
                );
            }
        }
    }

    #[test]
    fn starting_values_follow_variables_filter_and_arguments() {
        let modes = start(
            &[
                ("METRUM_DEBUG_FILTER", "road,terrain-full-lod1"),
                ("METRUM_DEBUG_TERRAIN_VISUAL", "Relief"),
                ("METRUM_DEBUG_TERRAIN_GRASS", "nonsense"),
                ("METRUM_DEBUG_BUILDING_SITES_VISUAL", "anything"),
                ("METRUM_DEBUG_PEDESTRIAN_VAT", "rest"),
            ],
            &["godot", "--", "--pedestrian-vat-debug", "UV"],
        );
        assert_eq!(
            modes,
            [
                ("terrain_visual", "relief"),
                ("terrain_grass", "off"),
                ("building_sites_visual", "material"),
                ("pedestrian_vat", "uv"),
                ("terrain_force_lod1", "on"),
                ("terrain_force_full_world", "on"),
                ("junction_connectors", "off"),
            ]
        );
        let modes = start(
            &[
                ("METRUM_DEBUG_TERRAIN_FORCE_LOD1", "1"),
                ("METRUM_DEBUG_BUILDING_SITES_VISUAL", "0"),
                ("METRUM_DEBUG_PEDESTRIAN_VAT", "rest"),
            ],
            &["--pedestrian-vat-debug=offsets"],
        );
        assert_eq!(modes[2], ("building_sites_visual", "off"));
        assert_eq!(modes[3], ("pedestrian_vat", "offsets"));
        assert_eq!(modes[4], ("terrain_force_lod1", "on"));
        assert_eq!(modes[5], ("terrain_force_full_world", "off"));
        assert!(start(&[], &[]).iter().all(|(_, value)| *value == "off"));
    }

    #[test]
    fn set_round_trips_and_refuses_unknown_names_and_values() {
        // Only this test switches runtime state, so parallel tests never observe it.
        let mode = lookup("terrain_grass");
        assert_eq!(set("terrain_grass", "macro"), Ok(2));
        assert_eq!(mode.index(), 2);
        assert_eq!(set("terrain_grass", "nonsense"), Err(Refusal::UnknownValue));
        assert_eq!(mode.index(), 2);
        assert_eq!(set("no_such_mode", "on"), Err(Refusal::UnknownMode));
        assert_eq!(set("terrain_grass", "off"), Ok(0));
        assert_eq!(mode.index(), 0);
    }
}
