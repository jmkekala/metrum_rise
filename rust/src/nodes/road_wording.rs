// SPDX-License-Identifier: GPL-2.0-only

//! Stateless bridge to the road wording (`ROAD-112` round V2): the player's text for a road
//! refusal key, from the one place the strings live (`nodes::sim::road_tool::wording`), for the
//! GDScript hosts that show a road verdict (`road_reason_text.gd`).

use crate::nodes::sim::road_tool::wording::{DEFAULT_REFUSAL, ReasonFacts, reason_text};
use godot::prelude::*;

/// The road verdicts' player text (`RoadWording.reason_text(verdict, fallback)`).
#[derive(GodotClass)]
#[class(init, base = RefCounted)]
pub struct RoadWording;

#[godot_api]
impl RoadWording {
    /// The text for `verdict`'s `invalid_reason`, quoting its `clearance_m`,
    /// `required_clearance_m`, `geometry_measured` and `geometry_limit`; `fallback` for a key
    /// without one.
    #[func]
    pub fn reason_text(verdict: VarDictionary, fallback: GString) -> GString {
        let reason = verdict
            .get("invalid_reason")
            .and_then(|v| v.try_to::<GString>().ok())
            .unwrap_or_default()
            .to_string();
        reason_text(&reason, &reason_facts(&verdict))
            .map_or(fallback, |text| GString::from(text.as_str()))
    }

    /// The text a refusal shows when its key has none of its own.
    #[func]
    pub fn default_refusal() -> GString {
        DEFAULT_REFUSAL.into()
    }
}

/// The numbers a road verdict dictionary quotes for its refusal (0 when absent; no measured
/// value when `geometry_measured` is absent).
fn reason_facts(verdict: &VarDictionary) -> ReasonFacts {
    // A number may arrive as a float or an int (a junction's road count).
    let number = |key: &str| {
        verdict.get(key).and_then(|v| {
            v.try_to::<f64>()
                .ok()
                .or_else(|| v.try_to::<i64>().ok().map(|n| n as f64))
        })
    };
    ReasonFacts {
        clearance_m: number("clearance_m").unwrap_or(0.0),
        required_clearance_m: number("required_clearance_m").unwrap_or(0.0),
        geometry_measured: number("geometry_measured"),
        geometry_limit: number("geometry_limit").unwrap_or(0.0),
    }
}
