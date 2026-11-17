// SPDX-License-Identifier: GPL-2.0-only

//! The player's text for the road tool and for road verdicts (`ROAD-112` round V2): the one place
//! these strings live (a later translation replaces this module's strings, nothing else). The
//! road tool's chips ([`label_text`]: each chip's value with its units, rounding and compass
//! letters, the draft hint, the Build and Cancel buttons and the notice's warning or refusal)
//! and the text of every road refusal key ([`reason_text`]), which `road_reason_text.gd` reads
//! through `RoadWording` for the other hosts (the Road Properties window, the road builder, the
//! junction pocket rows, the roundabout tool). GDScript only measures and draws these strings.

use std::fmt::Write;

use super::labels::{EndPart, HintStep, LabelKind, LabelTone, ToolLabel};
use crate::simulation::network::roadbed::NodeRole;

/// The numbers a refusal's text quotes, as the stroke preview's dictionary carries them
/// (`clearance_m`, `required_clearance_m`, `geometry_measured`, `geometry_limit`; 0 when absent).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ReasonFacts {
    /// The clearance or cover found, metres.
    pub clearance_m: f64,
    /// The clearance or cover needed, metres.
    pub required_clearance_m: f64,
    /// The measured quantity the rule checks (a radius, a grade, an angle, a count), if any.
    pub geometry_measured: Option<f64>,
    /// The rule's limit for it.
    pub geometry_limit: f64,
    /// A junction that does not fit (`ROAD-118`): the failing node's role and the role of the
    /// node across its short arm (`junction_roles`), with that road's length as measured and the
    /// shortest that fits as the limit (0 when none does).
    pub junction_roles: Option<[NodeRole; 2]>,
}

/// What a notice chip words: the draft's own refusal, else the standing preview's verdict.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NoticeFacts<'a> {
    /// The draft's refusal key (`road_tool_frame`'s `refusal`), "" without one.
    pub refusal: &'a str,
    /// The draft refusal's numbers: the draft's smallest radius (-1 for none) against the
    /// type's illegal radius.
    pub draft: ReasonFacts,
    /// The preview's refusal key (`invalid_reason`), "" without one.
    pub preview_reason: &'a str,
    /// The preview refusal's numbers.
    pub preview: ReasonFacts,
    /// A grade over the design tier the preview warns about, with that tier, as fractions.
    pub steep: Option<(f64, f64)>,
    /// Buildings the stroke would demolish.
    pub demolish_count: i64,
    /// The preview's plan is still being checked.
    pub checking: bool,
}

/// The characters beyond printable ASCII the texts here write (the HUD measures them too).
pub const CHIP_EXTRA_CHARACTERS: &str = "°·…";

// The compass letters by 45-degree sector, clockwise from north.
const COMPASS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];

/// Writes `label`'s text into `out` (cleared first): lengths "12.3 m" under 100 m, else "123
/// m"; radii "R 12.3 m" / "R 123 m"; angles and design speeds in whole units (truncated);
/// grades "1.5 %"; money "$12,345"; the bearing "NE 47°"; an end's height "+6.5 m bridge" (one
/// decimal, signed, and the part); the notice from `notice`.
pub fn label_text(label: &ToolLabel, notice: &NoticeFacts, out: &mut String) {
    out.clear();
    let value = label.value;
    // Writing into a `String` cannot fail.
    let _ = match label.kind {
        LabelKind::PieceLength | LabelKind::TotalLength if value >= 100.0 => {
            write!(out, "{value:.0} m")
        }
        LabelKind::PieceLength | LabelKind::TotalLength => write!(out, "{value:.1} m"),
        LabelKind::Radius if value >= 100.0 => write!(out, "R {value:.0} m"),
        LabelKind::Radius => write!(out, "R {value:.1} m"),
        LabelKind::Angle => write!(out, "{}°", value as i64),
        LabelKind::Slope => write!(out, "{value:.1} %"),
        LabelKind::Cost => {
            push_money(value, out);
            Ok(())
        }
        LabelKind::Bearing => write!(
            out,
            "{} {}°",
            COMPASS[((value / 45.0).round() as i64).rem_euclid(8) as usize],
            (value.round() as i64) % 360
        ),
        LabelKind::DesignSpeed => write!(out, "{} km/h", value as i64),
        LabelKind::Notice => {
            push_notice(label.tone, notice, out);
            Ok(())
        }
        LabelKind::Hint(step) => {
            out.push_str(hint_text(step));
            Ok(())
        }
        LabelKind::Build => {
            out.push_str("Build");
            Ok(())
        }
        LabelKind::Cancel => {
            out.push_str("Cancel");
            Ok(())
        }
        LabelKind::EndHeight(part) => write!(out, "{value:+.1} m {}", end_part_text(part)),
    };
}

/// The hint chip's text for an open draft's `step` (round V item 7: freeform; round SC: the
/// simple curve's and the S-curve's clicks, the user's wording of 2026-11-14).
pub fn hint_text(step: HintStep) -> &'static str {
    match step {
        HintStep::AddNode => "Click to add a node · click the last node or Enter to finish",
        HintStep::EditNodes => "Drag nodes or handles · Enter or click to build · Esc to cancel",
        HintStep::ControlPoint => "Click to place the control point · Esc to cancel",
        HintStep::StartDirection => "Click to set the start direction · Esc to cancel",
        HintStep::EndDirection => "Click to set the end direction · Esc to cancel",
        HintStep::PlaceEnd => "Click to place the end · Esc to cancel",
    }
}

// The part an end is on, as its height chip names it (round H2).
fn end_part_text(part: EndPart) -> &'static str {
    match part {
        EndPart::Ground => "ground",
        EndPart::Bridge => "bridge",
        EndPart::Cut => "cut",
        EndPart::Tunnel => "tunnel",
    }
}

// Money in whole units with thousands separators: "$1,234", "-$56".
fn push_money(value: f64, out: &mut String) {
    // The digits least significant first into a stack buffer (a `u64` has at most 20), so a
    // changed cost allocates nothing once `out` has grown (the tick rewrites the chips' words).
    let mut rest = value.abs().round() as u64;
    let mut digits = [0u8; 20];
    let mut len = 0;
    loop {
        digits[len] = b'0' + (rest % 10) as u8;
        len += 1;
        rest /= 10;
        if rest == 0 {
            break;
        }
    }
    out.push_str(if value < 0.0 { "-$" } else { "$" });
    for index in 0..len {
        if index > 0 && (len - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(char::from(digits[len - 1 - index]));
    }
}

// A notice: a refusal (the draft's own, else the preview's) or the preview's warnings, joined by
// " · " ("Placing…" for a stroke being sent with no warning).
fn push_notice(tone: LabelTone, notice: &NoticeFacts, out: &mut String) {
    if tone == LabelTone::Refused {
        let (reason, facts) = if notice.refusal.is_empty() {
            (notice.preview_reason, &notice.preview)
        } else {
            (notice.refusal, &notice.draft)
        };
        out.push_str(&reason_text(reason, facts).unwrap_or_else(|| DEFAULT_REFUSAL.into()));
        return;
    }
    let part = |out: &mut String| {
        if !out.is_empty() {
            out.push_str(" · ");
        }
    };
    // A grade above the design tier is shown, not refused (ROAD-68 question 37).
    if let Some((grade, limit)) = notice.steep {
        part(out);
        let _ = write!(
            out,
            "steep {:.0}% (design {:.0}%)",
            grade * 100.0,
            limit * 100.0
        );
    }
    if notice.demolish_count > 0 {
        part(out);
        let plural = if notice.demolish_count == 1 { "" } else { "s" };
        let _ = write!(out, "demolishes {} building{plural}", notice.demolish_count);
    }
    if notice.checking {
        part(out);
        out.push_str("checking road");
    }
    if out.is_empty() {
        out.push_str("Placing…");
    }
}

/// The text for a refusal the preview names without one of its own.
pub const DEFAULT_REFUSAL: &str = "Road cannot be placed";

// A junction that does not fit, by the place (`ROAD-118` round JS1): the two nodes across the
// short road, its length and the shortest that fits. "Junction only 10.0 m from the bend; needs
// 17.0 m", "Road end only 6.0 m from the junction; needs 9.5 m". `None` without the place.
fn spacing_text(facts: &ReasonFacts) -> Option<String> {
    let [node, far] = facts.junction_roles?;
    let length_m = facts.geometry_measured?;
    let name = |role: NodeRole| match role {
        NodeRole::Junction => "junction",
        NodeRole::Bend => "bend",
        NodeRole::RoadEnd => "road end",
        NodeRole::Joint => "joint in the road",
    };
    // The road end is what stands too close; two junctions are apart.
    let (subject, object) = if far == NodeRole::RoadEnd { (far, node) } else { (node, far) };
    let mut text = if subject == object {
        format!("{}s only {length_m:.1} m apart", capitalized(name(subject)))
    } else {
        format!(
            "{} only {length_m:.1} m from the {}",
            capitalized(name(subject)),
            name(object)
        )
    };
    if facts.geometry_limit > 0.0 {
        let _ = write!(text, "; needs {:.1} m", facts.geometry_limit);
    }
    Some(text)
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
        .unwrap_or_default()
}

/// The player's text for Rust's road refusal key `reason` with the numbers it quotes, `None`
/// for a key without one (the caller's fallback).
pub fn reason_text(reason: &str, facts: &ReasonFacts) -> Option<String> {
    let measured = facts.geometry_measured.unwrap_or(0.0);
    if matches!(reason, "junction_corner_does_not_fit" | "junction_arm_too_short")
        && let Some(text) = spacing_text(facts)
    {
        return Some(text);
    }
    Some(match reason {
        "bridge_clearance" => format!("Bridge clearance {:.1} m / {:.1} m", facts.clearance_m, facts.required_clearance_m),
        "tunnel_cover" => format!("Tunnel cover {:.1} m / {:.1} m: draw the ends outside the hill", facts.clearance_m, facts.required_clearance_m),
        "tunnel_approach_too_short" => format!("Tunnel approach {:.1} m, needs {:.1} m: start further from the hill", measured, facts.geometry_limit),
        "tunnel_too_close" => format!("Too close above a tunnel: {:.1} m / {:.1} m over its deck", facts.clearance_m, facts.required_clearance_m),
        "bridge_too_low" => format!("Too low under a bridge: {:.2} m / {:.2} m deck to deck", facts.clearance_m, facts.required_clearance_m),
        "tunnel_join" => "Roads cannot join a tunnel inside its bore: join it at a portal".into(),
        // A graded draft (`ROAD-112` rounds H, I): the segmentation's and the batch's refusals.
        "cut_too_deep" => "Too deep for a cutting (over 5 m) and too little ground over it for a tunnel: raise or lower the road".into(),
        "batch_too_large" => "Too long for one road draft: draw it in shorter pieces".into(),
        "tunnel_crossing" => "Tunnels cannot cross other tunnels or meet roads underground".into(),
        "junction_in_trench" => format!("Junction {:.1} m below the ground in a tunnel approach: join where the road meets the ground", measured),
        "tunnel_under_water" => "Tunnels cannot pass under water".into(),
        "water_requires_bridge" => "Water crossing requires a bridge".into(),
        "field_overlap" => "Road overlaps a farm field".into(),
        "extraction_area_overlap" => "Road overlaps a mine's extraction area".into(),
        "plotted_building_overlap" => "Road overlaps a placed building's lot".into(),
        "surface_geometry_invalid" => "Road surface compilation failed".into(),
        "terrain_clip_missing_outer_boundary_owner" => "Road terrain boundary could not be built".into(),
        "parcel_overlap" => "Road overlaps existing zoning parcels".into(),
        "road_grade_change" => "Road bends too sharply up or down".into(),
        "road_too_steep" => match facts.geometry_measured {
            None => "Road too steep".into(),
            Some(measured) => format!(
                "Road too steep: {:.0}%, at most {:.0}%",
                measured * 100.0,
                facts.geometry_limit * 100.0
            ),
        },
        "road_tight_curve" => format!("Curve too tight: radius {:.1} m, needs {:.1} m", measured, facts.geometry_limit),
        "junction_arm_angle" => format!("Roads meet at {:.0}°; junction arms need {:.0}°", measured, facts.geometry_limit),
        "junction_corner_does_not_fit" => "Junction corner does not fit here".into(),
        "junction_core_not_simple" => "Junction roads overlap; widen the angle between them".into(),
        "junction_band_mismatch" => "These road types cannot meet at a junction yet".into(),
        "junction_arm_too_short" => "Road too short for its junction".into(),
        "junction_profile_mismatch" => "Junction heights do not fit; adjust the road grade".into(),
        "junction_layout_invalid" => "Junction could not be laid out".into(),
        "junction_degree" => format!("Too many roads at one junction ({}, at most {})", measured as i64, facts.geometry_limit as i64),
        "same_node_connection" => "Road endpoints connect to the same junction".into(),
        "too_short" => "Road too short".into(),
        "validation_unavailable" => "Road check unavailable".into(),
        "invalid_road" => "The road no longer exists".into(),
        "cross_section_not_road" => "A walkway cannot become a road: draw a road in its place".into(),
        "too_many_lanes" => "At most 4 lanes in each direction".into(),
        "cross_section_cannot_change_transit_mode" => "A road cannot become a walkway".into(),
        "cross_section_unchanged" => "No change".into(),
        "sidewalk_under_door" => "A building's door is on this sidewalk: remove the building first".into(),
        "edit_tunnel_width" => "Widen a tunnel by drawing it again".into(),
        "cross_section_too_wide_for_curve" => "Too wide for this curve".into(),
        // Turn pockets (`ROAD-95`): the pocket edit and the strokes and edits it refuses.
        "pocket_road_too_short" => "Road too short for a pocket here".into(),
        "pocket_movement_missing" => "The junction has no such turn from this road".into(),
        "pocket_junction_whitelisted" => "Clear the junction's turn rules first".into(),
        "pocket_median_gap" => "The median cannot continue past a pocket here".into(),
        "pocket_requires_surface_road" => "Turn pockets are for roads on the ground".into(),
        "pocket_lane_limit" => "At most 4 lanes in each direction, the pocket included".into(),
        "pocket_requires_wide_median" => "A left turn pocket needs a raised median at least 3.8 m wide".into(),
        "invalid_pocket_dimensions" => "Pocket length 20 to 300 m, in steps of 10".into(),
        "pocket_unchanged" => "No change".into(),
        "pocket_transition_changed" => "The pocket's road has changed: remove the pocket and add it again".into(),
        "pocket_edit_first" => "Remove the turn pocket first".into(),
        "stroke_splits_pocket" => "Roads cannot cross or join a turn pocket: remove it first".into(),
        "roundabout_fixed" => "A roundabout's ring cannot be changed".into(),
        "roundabout_crossed" => "Roads cannot cross a roundabout: end the road on its ring".into(),
        "roundabout_island" => "Roads cannot go on a roundabout's island".into(),
        "roundabout_arm_node_taken" => "This point of the roundabout already has a road".into(),
        "roundabout_neighbour_nodes" => "A road cannot link two neighbouring points of a roundabout".into(),
        "roundabout_arms_too_close" => "Arms need more room. Increase the diameter".into(),
        "roundabout_overlaps_road" => "Another road occupies this site".into(),
        "roundabout_requires_three_to_six_arms" => "Choose a junction with three to six roads".into(),
        "roundabout_requires_ordinary_surface_arms" => "Every road here must be an ordinary surface road".into(),
        "roundabout_has_lane_restrictions" => "Remove the junction's lane rules first".into(),
        "roundabout_arm_has_pocket" => "Remove the turn pockets at this junction first".into(),
        "roundabout_arm_too_short" => "A road here is too short for this diameter".into(),
        "roundabout_replacement_too_large" => "A junction takes a roundabout of at most 60 m".into(),
        "invalid_roundabout_junction" => "This junction no longer exists".into(),
        "invalid_roundabout_dimensions" => "This diameter does not suit this roundabout".into(),
        "median_requires_both_directions" => "A median needs lanes in both directions".into(),
        "median_requires_vehicle_road" => "Only roads with lanes can have a median".into(),
        "median_width_out_of_range" => "Median width out of range for its style".into(),
        "unknown_median_style" => "Unknown median style".into(),
        // Road types and their library (`ROAD-101`): the layout check, names, library edits.
        "layout_footpath_alone" => "A footpath is a walkway on its own".into(),
        "layout_sidewalks" => "A sidewalk goes at an end of the road".into(),
        "sidewalks_out_of_range" => "Unknown sidewalk sides".into(),
        "layout_two_medians" => "One median at most".into(),
        "layout_lane_order" => "Lanes against traffic go on the left".into(),
        "layout_no_car_lane" => "A road needs at least one car lane".into(),
        "layout_lanes_split" => "A median sits between the two directions, not inside one".into(),
        "layout_median_at_edge" => "A median sits between the two directions".into(),
        // Kerbside parking (`PARK-05`).
        "layout_parking_at_kerb" => "A parking lane goes at the kerb, the outermost part of the road".into(),
        "layout_parking_needs_sidewalk" => "A parking lane needs a sidewalk on its side".into(),
        "parking_requires_vehicle_road" => "Only roads with lanes can have parking".into(),
        "parking_code_out_of_range" => "Unknown parking setting".into(),
        "layout_unknown_part" => "Unknown road part".into(),
        "speed_out_of_range" => "Speed between 20 and 120 km/h, in steps of 10".into(),
        "name_empty" => "Enter a name".into(),
        "name_too_long" => "At most 40 characters".into(),
        "name_invalid_characters" => "No line breaks or control characters".into(),
        "name_taken" => "Another road type has this name".into(),
        "road_type_builtin" => "Built-in road types cannot be changed".into(),
        "road_type_unknown" => "This road type no longer exists".into(),
        "road_type_duplicate_id" => "Another road type in the library has the same id".into(),
        "entry_unreadable" => "This road type could not be read from the library".into(),
        "library_unreadable" => "The road library file could not be read: fix or remove it to save changes".into(),
        "library_newer_version" => "The road library is from a newer version of the game: read-only".into(),
        "library_write_failed" => "The road library could not be saved".into(),
        "node_piece_failed" => "A junction at the road's end cannot fit this width".into(),
        "road_commit_rejected" => "Road could not be built; adjust the road and try again".into(),
        // The road tool's draft refusals (`road_tool_frame`'s `refusal`, `ROAD-112`).
        "fold_back" => "The curve folds back on itself: move the control point".into(),
        "full_turn" => "The end lies straight behind the start's direction".into(),
        "no_biarc" => "No S-curve joins these two directions: move a tangent point".into(),

        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::guide::DVec2;
    use godot::prelude::Vector3;

    fn text(kind: LabelKind, value: f64, tone: LabelTone, notice: &NoticeFacts) -> String {
        let label = ToolLabel {
            kind,
            value,
            anchor: Vector3::ZERO,
            side: DVec2::X,
            tone,
        };
        let mut out = "stale".to_string();
        label_text(&label, notice, &mut out);
        out
    }

    // Every chip kind's text: units, rounding by size, truncated whole degrees and km/h, money
    // with separators, compass letters, the hint and the buttons.
    #[test]
    fn chip_texts() {
        let none = NoticeFacts::default();
        let normal = LabelTone::Normal;
        for (kind, value, expected) in [
            (LabelKind::PieceLength, 42.25, "42.2 m"),
            (LabelKind::PieceLength, 99.94, "99.9 m"),
            (LabelKind::TotalLength, 153.4, "153 m"),
            (LabelKind::Radius, 37.26, "R 37.3 m"),
            (LabelKind::Radius, 250.0, "R 250 m"),
            (LabelKind::Angle, 89.9, "89°"),
            (LabelKind::Slope, -3.04, "-3.0 %"),
            (LabelKind::Cost, 1_234_567.4, "$1,234,567"),
            (LabelKind::Cost, 999.6, "$1,000"),
            (LabelKind::Cost, -56.0, "-$56"),
            (LabelKind::Bearing, 47.0, "NE 47°"),
            (LabelKind::Bearing, 359.6, "N 0°"),
            (LabelKind::Bearing, 202.4, "S 202°"),
            (LabelKind::DesignSpeed, 49.7, "49 km/h"),
            (
                LabelKind::Hint(HintStep::EditNodes),
                0.0,
                "Drag nodes or handles · Enter or click to build · Esc to cancel",
            ),
            (
                LabelKind::Hint(HintStep::StartDirection),
                0.0,
                "Click to set the start direction · Esc to cancel",
            ),
            (LabelKind::Build, 0.0, "Build"),
            (LabelKind::Cancel, 0.0, "Cancel"),
        ] {
            assert_eq!(
                text(kind, value, normal, &none),
                expected,
                "{kind:?} {value}"
            );
        }
    }

    // A notice words the draft's refusal before the preview's, the preview's warnings joined,
    // and a stroke being sent without one.
    #[test]
    fn notice_texts() {
        let draft = NoticeFacts {
            refusal: "road_tight_curve",
            draft: ReasonFacts {
                geometry_measured: Some(4.0),
                geometry_limit: 12.5,
                ..ReasonFacts::default()
            },
            preview_reason: "field_overlap",
            ..NoticeFacts::default()
        };
        let refused = LabelTone::Refused;
        assert_eq!(
            text(LabelKind::Notice, 0.0, refused, &draft),
            "Curve too tight: radius 4.0 m, needs 12.5 m"
        );
        let preview = NoticeFacts {
            refusal: "",
            ..draft
        };
        assert_eq!(
            text(LabelKind::Notice, 0.0, refused, &preview),
            "Road overlaps a farm field"
        );
        let unknown = NoticeFacts {
            preview_reason: "no_such_key",
            ..preview
        };
        assert_eq!(
            text(LabelKind::Notice, 0.0, refused, &unknown),
            DEFAULT_REFUSAL
        );
        let warning = LabelTone::Warning;
        let warned = NoticeFacts {
            steep: Some((0.112, 0.08)),
            demolish_count: 2,
            checking: true,
            ..NoticeFacts::default()
        };
        assert_eq!(
            text(LabelKind::Notice, 0.0, warning, &warned),
            "steep 11% (design 8%) · demolishes 2 buildings · checking road"
        );
        let one = NoticeFacts {
            demolish_count: 1,
            ..NoticeFacts::default()
        };
        assert_eq!(
            text(LabelKind::Notice, 0.0, warning, &one),
            "demolishes 1 building"
        );
        assert_eq!(
            text(LabelKind::Notice, 0.0, warning, &NoticeFacts::default()),
            "Placing…"
        );
    }

    // The refusal table quotes its numbers; a key without text has none.
    #[test]
    fn reason_texts() {
        let facts = ReasonFacts {
            clearance_m: 3.25,
            required_clearance_m: 4.5,
            geometry_measured: Some(7.0),
            geometry_limit: 6.0,
            junction_roles: None,
        };
        let say = |reason| reason_text(reason, &facts);
        assert_eq!(
            say("bridge_too_low").as_deref(),
            Some("Too low under a bridge: 3.25 m / 4.50 m deck to deck")
        );
        assert_eq!(
            say("junction_degree").as_deref(),
            Some("Too many roads at one junction (7, at most 6)")
        );
        assert_eq!(
            say("road_too_steep").as_deref(),
            Some("Road too steep: 700%, at most 600%")
        );
        let unmeasured = ReasonFacts {
            geometry_measured: None,
            ..facts
        };
        assert_eq!(
            reason_text("road_too_steep", &unmeasured).as_deref(),
            Some("Road too steep")
        );
        assert_eq!(say("no_such_key"), None);
        assert_eq!(say(""), None);
    }

    // A junction that does not fit names the place: the two nodes across the short road, its
    // length and the shortest that fits (`ROAD-118` round JS1).
    #[test]
    fn spacing_texts_name_the_place() {
        let facts = |roles, measured, limit| ReasonFacts {
            geometry_measured: Some(measured),
            geometry_limit: limit,
            junction_roles: Some(roles),
            ..ReasonFacts::default()
        };
        let say = |reason, roles, measured, limit| {
            reason_text(reason, &facts(roles, measured, limit)).unwrap()
        };
        use NodeRole::*;
        let corner = "junction_corner_does_not_fit";
        assert_eq!(
            say(corner, [Junction, Bend], 10.04, 17.0),
            "Junction only 10.0 m from the bend; needs 17.0 m"
        );
        assert_eq!(
            say("junction_arm_too_short", [Junction, RoadEnd], 6.0, 9.5),
            "Road end only 6.0 m from the junction; needs 9.5 m"
        );
        assert_eq!(
            say(corner, [Junction, Junction], 12.0, 17.0),
            "Junctions only 12.0 m apart; needs 17.0 m"
        );
        assert_eq!(
            say(corner, [Bend, Joint], 5.0, 0.0),
            "Bend only 5.0 m from the joint in the road"
        );
        // Without the place, the key's own words, which ask nothing of the player.
        assert_eq!(
            reason_text(corner, &ReasonFacts::default()).as_deref(),
            Some("Junction corner does not fit here")
        );
    }
}
