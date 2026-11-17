// SPDX-License-Identifier: GPL-2.0-only

//! Stateless bridge to the road wording (`ROAD-112` round V2): the player's text for a road
//! refusal key, from the one place the strings live (`nodes::sim::road_tool::wording`), for the
//! GDScript hosts that show a road verdict (`road_reason_text.gd`).

use crate::nodes::sim::road_tool::wording::{DEFAULT_REFUSAL, ReasonFacts, reason_text};
use crate::simulation::network::road_edit::{JunctionPlace, RoadPreviewValidation};
use crate::simulation::network::roadbed::NodeRole;
use godot::prelude::*;

/// The road verdicts' player text (`RoadWording.reason_text(verdict, fallback)`).
#[derive(GodotClass)]
#[class(init, base = RefCounted)]
pub struct RoadWording;

#[godot_api]
impl RoadWording {
    /// The text for `verdict`'s `invalid_reason`, quoting its `clearance_m`,
    /// `required_clearance_m`, `geometry_measured` and `geometry_limit` and naming its
    /// `junction_roles` (`ROAD-118`); `fallback` for a key without one.
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
        junction_roles: junction_roles(verdict),
    }
}

fn junction_roles(verdict: &VarDictionary) -> Option<[NodeRole; 2]> {
    let roles = verdict.get("junction_roles")?.try_to::<Vector2i>().ok()?;
    Some([
        NodeRole::from_code(i64::from(roles.x))?,
        NodeRole::from_code(i64::from(roles.y))?,
    ])
}

/// Writes a verdict's refusal places into its dictionary (`ROAD-118` round JS1): the junction
/// place (`junction_node`, -1 for a new node; `junction_position`; `junction_arm_edge`, -1 for a
/// stroke piece; `junction_roles` as role codes) and each end's road
/// (`start_endpoint_edge`, `start_endpoint_station_m`, likewise `end_`). Keys only where the
/// verdict has them.
pub(crate) fn write_refusal_places(validation: &RoadPreviewValidation, dict: &mut VarDictionary) {
    if let Some(place) = validation.junction_place {
        dict.set("junction_node", place.node.map_or(-1, i64::from));
        dict.set("junction_position", place.position);
        dict.set(
            "junction_arm_edge",
            place.arm_edge.map_or(-1, |edge| edge as i64),
        );
        dict.set(
            "junction_roles",
            Vector2i::new(place.roles[0].code(), place.roles[1].code()),
        );
    }
    for (prefix, end) in ["start", "end"].into_iter().zip(validation.endpoint_edges) {
        if let Some((edge, station_m)) = end {
            dict.set(format!("{prefix}_endpoint_edge").as_str(), edge as i64);
            dict.set(format!("{prefix}_endpoint_station_m").as_str(), station_m);
        }
    }
}

/// The refusal places [`write_refusal_places`] wrote: the junction place and each end's road.
pub(crate) fn read_refusal_places(
    dict: &VarDictionary,
) -> (Option<JunctionPlace>, [Option<(usize, f32)>; 2]) {
    let int = |key: &str| dict.get(key).and_then(|v| v.try_to::<i64>().ok());
    let place = (|| {
        Some(JunctionPlace {
            node: u32::try_from(int("junction_node")?).ok(),
            position: dict.get("junction_position")?.try_to::<Vector3>().ok()?,
            arm_edge: usize::try_from(int("junction_arm_edge")?).ok(),
            roles: junction_roles(dict)?,
        })
    })();
    let end = |prefix: &str| {
        let edge = usize::try_from(int(&format!("{prefix}_endpoint_edge"))?).ok()?;
        let station_m = dict
            .get(format!("{prefix}_endpoint_station_m").as_str())?
            .try_to::<f32>()
            .ok()?;
        Some((edge, station_m))
    };
    (place, [end("start"), end("end")])
}
