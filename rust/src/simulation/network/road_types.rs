// SPDX-License-Identifier: GPL-2.0-only

//! Road types (`ROAD-101`): a road's layout as the player composes it, an ordered list of parts
//! left to right looking along the road's forward direction, and the one check that turns it
//! into the cross-section an edge stores (forward lanes, backward lanes, median) or refuses it
//! with a stable key. The built-in types the Roads menu offers are one table here; the player's
//! own types live in the per-user library file ([`library`]).
//!
//! A type is tool configuration, not simulation state: a built road keeps its own lanes and
//! median in its edge, so no save, undo record or simulation system names a type, and editing a
//! type changes only the roads drawn after it. Engine-free: the Godot API resolves the file path
//! and converts the parts (`nodes::simulation_node::network_api::road_types`).

#[cfg(test)]
mod tests;

use super::graph::{Edge, MAX_LANES_PER_DIRECTION, MedianStyle, RoadMedian};
use super::roadbed::FOOTPATH_MIN_WIDTH_M;
use super::types::TransitFlags;
use crate::config;

/// One part of a road layout, as the road builder shows it from the side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RoadPart {
    /// A sidewalk with its kerb, `config::SIDEWALK_WIDTH` wide; a car road has one at each end.
    Sidewalk,
    /// A car lane, `config::LANE_WIDTH` wide.
    CarLane {
        /// Whether the lane carries traffic along the road's forward direction (else against it).
        forward: bool,
    },
    /// The centre median, between the two directions' lanes.
    Median(RoadMedian),
    /// A walkway's single footpath band: valid only as the whole layout.
    Footpath,
}

impl RoadPart {
    /// The part's width across the road in metres: a sidewalk 1.5, a lane 3.5, a median its own
    /// width, a footpath the walkway's 2. `O(1)`.
    pub fn width_m(self) -> f32 {
        match self {
            Self::Sidewalk => config::SIDEWALK_WIDTH,
            Self::CarLane { .. } => config::LANE_WIDTH,
            Self::Median(median) => median.width_m,
            Self::Footpath => FOOTPATH_MIN_WIDTH_M,
        }
    }
}

/// What a valid layout builds: the lane counts and median an edge stores. Every car road also
/// carries a sidewalk on each side; a walkway (0 + 0 lanes) is one footpath band.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoadCrossSection {
    /// Lanes along the road's forward direction.
    pub fwd_lanes: u8,
    /// Lanes against it.
    pub bkw_lanes: u8,
    /// The median between the two directions, if any.
    pub median: Option<RoadMedian>,
}

impl RoadCrossSection {
    /// A walkway: no car lane, no median.
    pub const WALKWAY: Self = Self::undivided(0, 0);

    const fn undivided(fwd_lanes: u8, bkw_lanes: u8) -> Self {
        Self {
            fwd_lanes,
            bkw_lanes,
            median: None,
        }
    }

    /// Whether this is a walkway (no car lane).
    pub fn is_walkway(self) -> bool {
        self.fwd_lanes == 0 && self.bkw_lanes == 0
    }

    /// The layout this cross-section is drawn from, left to right along the forward direction:
    /// a walkway's footpath, else a sidewalk, the backward lanes, the median, the forward lanes
    /// and a sidewalk (right-hand traffic; mirrored when driving on the left). `O(lanes)`.
    pub fn parts(self) -> Vec<RoadPart> {
        if self.is_walkway() {
            return vec![RoadPart::Footpath];
        }
        let (left, right) = directions_left_to_right(self.fwd_lanes, self.bkw_lanes);
        let left_lane = RoadPart::CarLane {
            forward: config::DRIVE_ON_LEFT,
        };
        let right_lane = RoadPart::CarLane {
            forward: !config::DRIVE_ON_LEFT,
        };
        let mut parts = Vec::with_capacity(usize::from(left + right) + 3);
        parts.push(RoadPart::Sidewalk);
        parts.extend((0..left).map(|_| left_lane));
        parts.extend(self.median.map(RoadPart::Median));
        parts.extend((0..right).map(|_| right_lane));
        parts.push(RoadPart::Sidewalk);
        parts
    }

    /// The name a new type of this cross-section is offered: "2+2", "2+2 Grass 3 m", "1+0" for a
    /// one-way road, "Walkway" for a walkway.
    pub fn base_name(self) -> String {
        if self.is_walkway() {
            return "Walkway".to_string();
        }
        let lanes = format!("{}+{}", self.fwd_lanes, self.bkw_lanes);
        match self.median {
            None => lanes,
            Some(median) => {
                let style = match median.style {
                    MedianStyle::Painted => "Painted",
                    MedianStyle::KerbedGrass => "Grass",
                    MedianStyle::KerbedPaving => "Paving",
                };
                // f32's shortest display: "3" for 3.0, "2.5" for 2.5.
                format!("{lanes} {style} {} m", median.width_m)
            }
        }
    }
}

// The lane counts left and right of the road's centre line along its forward direction:
// backward lanes sit left in right-hand traffic.
fn directions_left_to_right(fwd_lanes: u8, bkw_lanes: u8) -> (u8, u8) {
    if config::DRIVE_ON_LEFT {
        (fwd_lanes, bkw_lanes)
    } else {
        (bkw_lanes, fwd_lanes)
    }
}

/// Why a layout cannot be built: a stable refusal key (worded by Godot's `road_reason_text.gd`)
/// and the index of the part it is about, when one part is to blame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayoutRefusal {
    /// The stable refusal key.
    pub reason: &'static str,
    /// The offending part's index in the layout.
    pub part: Option<usize>,
}

impl LayoutRefusal {
    const fn new(reason: &'static str, part: Option<usize>) -> Self {
        Self { reason, part }
    }
}

/// Checks a layout and returns the cross-section it builds, or the first rule it breaks, in this
/// order:
/// - a footpath anywhere but as the whole layout: `layout_footpath_alone`;
/// - a sidewalk missing at either end, or one inside the road: `layout_sidewalks`;
/// - two medians: `layout_two_medians`;
/// - a backward lane right of a forward one (right-hand traffic): `layout_lane_order`;
/// - no car lane: `layout_no_car_lane`;
/// - more than [`MAX_LANES_PER_DIRECTION`] in a direction: `too_many_lanes`;
/// - the median rules every road shares (`Edge::validate_median`): `median_width_out_of_range`,
///   `median_requires_both_directions` (one-way);
/// - a median inside one direction's lanes: `layout_lanes_split`; outside both:
///   `layout_median_at_edge`.
///
/// `O(parts)`, allocation-free. Never reorders: a layout outside the order is refused.
pub fn check_layout(parts: &[RoadPart]) -> Result<RoadCrossSection, LayoutRefusal> {
    if let Some(at) = parts.iter().position(|part| *part == RoadPart::Footpath) {
        return if parts.len() == 1 {
            Ok(RoadCrossSection::WALKWAY)
        } else {
            Err(LayoutRefusal::new("layout_footpath_alone", Some(at)))
        };
    }
    let ends = (parts.first(), parts.last());
    if parts.len() < 2 || ends != (Some(&RoadPart::Sidewalk), Some(&RoadPart::Sidewalk)) {
        return Err(LayoutRefusal::new("layout_sidewalks", None));
    }
    let inside = &parts[1..parts.len() - 1];
    // Lanes counted left and right of the centre line, and how many of each the median follows.
    let (mut left, mut right) = (0usize, 0usize);
    let mut median: Option<(usize, RoadMedian, usize, usize)> = None;
    for (offset, part) in inside.iter().enumerate() {
        let at = Some(offset + 1);
        match *part {
            RoadPart::Sidewalk => return Err(LayoutRefusal::new("layout_sidewalks", at)),
            RoadPart::Median(found) => {
                if median.is_some() {
                    return Err(LayoutRefusal::new("layout_two_medians", at));
                }
                median = Some((offset + 1, found, left, right));
            }
            RoadPart::CarLane { forward } => {
                if forward == config::DRIVE_ON_LEFT {
                    if right > 0 {
                        return Err(LayoutRefusal::new("layout_lane_order", at));
                    }
                    left += 1;
                } else {
                    right += 1;
                }
            }
            // Refused above.
            RoadPart::Footpath => {}
        }
    }
    if left + right == 0 {
        return Err(LayoutRefusal::new("layout_no_car_lane", None));
    }
    let max = usize::from(MAX_LANES_PER_DIRECTION);
    if left > max || right > max {
        return Err(LayoutRefusal::new("too_many_lanes", None));
    }
    // Both counts are at most MAX_LANES_PER_DIRECTION.
    let (left, right) = (left as u8, right as u8);
    let (fwd_lanes, bkw_lanes) = directions_left_to_right(left, right);
    let section = RoadCrossSection {
        fwd_lanes,
        bkw_lanes,
        median: median.map(|(_, median, _, _)| median),
    };
    if let Some((at, found, left_before, right_before)) = median {
        let at = Some(at);
        Edge::validate_median(
            TransitFlags::CAR | TransitFlags::FOOT,
            fwd_lanes,
            bkw_lanes,
            Some(found),
        )
        .map_err(|reason| LayoutRefusal::new(reason, at))?;
        // The lanes already run left then right, so the median sits between the directions
        // exactly when every left lane precedes it and no right lane does.
        let (left_before, right_before) = (left_before as u8, right_before as u8);
        if (0 < left_before && left_before < left) || (0 < right_before && right_before < right) {
            return Err(LayoutRefusal::new("layout_lanes_split", at));
        }
        if left_before != left || right_before != 0 {
            return Err(LayoutRefusal::new("layout_median_at_edge", at));
        }
    }
    Ok(section)
}

/// A layout's widths in metres: each part's, the carriageway (the parts between the sidewalks:
/// the edge's stored width for a valid layout) and the total footprint, sidewalks included.
/// Defined for refused layouts too, so the builder can draw them. `O(parts)`.
pub fn layout_widths(parts: &[RoadPart]) -> (Vec<f32>, f32, f32) {
    let widths: Vec<f32> = parts.iter().map(|part| part.width_m()).collect();
    let total = widths.iter().sum();
    let sidewalks: f32 = parts
        .iter()
        .filter(|part| **part == RoadPart::Sidewalk)
        .map(|part| part.width_m())
        .sum();
    (widths, total - sidewalks, total)
}

/// A type the Roads menu offers in every game, in its button order.
#[derive(Clone, Copy, Debug)]
pub struct BuiltinRoadType {
    /// Stable id (`builtin:...`), never stored in a save.
    pub id: &'static str,
    /// The menu's label.
    pub name: &'static str,
    /// What it builds.
    pub cross_section: RoadCrossSection,
}

/// The built-in road types, in the Roads menu's order: Walkway, 2-Lane Road (1 + 1), 4-Lane
/// Road (2 + 2), One-Way 1L (1 + 0), One-Way 2L (2 + 0). None has a median; the player's types
/// add those.
pub const BUILTIN_ROAD_TYPES: [BuiltinRoadType; 5] = [
    builtin("builtin:walkway", "Walkway", 0, 0),
    builtin("builtin:road_2l", "2-Lane Road", 1, 1),
    builtin("builtin:road_4l", "4-Lane Road", 2, 2),
    builtin("builtin:one_way_1l", "One-Way 1L", 1, 0),
    builtin("builtin:one_way_2l", "One-Way 2L", 2, 0),
];

const fn builtin(id: &'static str, name: &'static str, fwd: u8, bkw: u8) -> BuiltinRoadType {
    BuiltinRoadType {
        id,
        name,
        cross_section: RoadCrossSection::undivided(fwd, bkw),
    }
}

/// Longest type name, in Unicode scalar values (it labels a menu button).
pub const MAX_NAME_CHARS: usize = 40;

/// A type name as the player typed it, trimmed, or why it cannot be one: `name_empty`,
/// `name_too_long` (over [`MAX_NAME_CHARS`]), `name_invalid_characters` (a control character
/// or line break). Uniqueness is the library's ([`same_name`]). `O(name)`, allocation-free.
pub fn clean_name(name: &str) -> Result<&str, &'static str> {
    let name = name.trim();
    if name.is_empty() {
        return Err("name_empty");
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err("name_too_long");
    }
    if name.chars().any(char::is_control) {
        return Err("name_invalid_characters");
    }
    Ok(name)
}

/// Whether two names are the same type name: trimmed, equal in Unicode lowercase.
/// Allocation-free.
pub fn same_name(a: &str, b: &str) -> bool {
    fn lower(name: &str) -> impl Iterator<Item = char> + '_ {
        name.trim().chars().flat_map(char::to_lowercase)
    }
    lower(a).eq(lower(b))
}

/// The cross-section's [`RoadCrossSection::base_name`], or the first of "<base> (2)",
/// "<base> (3)", ... that `taken` does not hold. `O(taken names x attempts)` on a UI action.
pub fn suggested_name(section: RoadCrossSection, taken: impl Fn(&str) -> bool) -> String {
    let base = section.base_name();
    if !taken(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base} ({n})"))
        .find(|name| !taken(name))
        .expect("finitely many names are taken")
}

/// Why a road type is unusable or a library request was refused: a stable key and, for the
/// log, a detail (the offending part's index, a parse error, a file error).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoadTypeRefusal {
    /// The stable refusal key.
    pub reason: &'static str,
    /// What it is about, for logs and debugging; not shown as the player's text.
    pub detail: String,
}

impl RoadTypeRefusal {
    pub(crate) fn new(reason: &'static str, detail: impl Into<String>) -> Self {
        Self {
            reason,
            detail: detail.into(),
        }
    }
}

impl From<LayoutRefusal> for RoadTypeRefusal {
    fn from(refusal: LayoutRefusal) -> Self {
        Self::new(
            refusal.reason,
            refusal
                .part
                .map_or_else(String::new, |part| format!("part {part}")),
        )
    }
}

/// A road type as the Roads menu and the builder list it: a built-in or one of the player's.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadType {
    /// Stable id: `builtin:...` or `custom:<n>`.
    pub id: String,
    /// The player's name for it.
    pub name: String,
    /// Whether it is one of [`BUILTIN_ROAD_TYPES`] (not editable).
    pub builtin: bool,
    /// Its layout; empty for a library entry that could not be read.
    pub parts: Vec<RoadPart>,
    /// What it builds, or why it is unusable (kept in the library file, listed, never drawn).
    pub verdict: Result<RoadCrossSection, RoadTypeRefusal>,
}

impl RoadType {
    pub(crate) fn from_builtin(builtin: &BuiltinRoadType) -> Self {
        Self {
            id: builtin.id.to_string(),
            name: builtin.name.to_string(),
            builtin: true,
            parts: builtin.cross_section.parts(),
            verdict: Ok(builtin.cross_section),
        }
    }

    /// The cross-section, when the type is usable.
    pub fn usable(&self) -> Option<RoadCrossSection> {
        self.verdict.as_ref().ok().copied()
    }
}

/// The first usable type in `types`' order whose cross-section is `section`: what Road
/// Properties shows as a road's type (duplicate layouts resolve to the first). `O(types)`.
pub fn type_of(types: &[RoadType], section: RoadCrossSection) -> Option<&RoadType> {
    types.iter().find(|ty| ty.usable() == Some(section))
}
