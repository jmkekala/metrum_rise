// SPDX-License-Identifier: GPL-2.0-only

//! Road types (`ROAD-101`): a road's layout as the player composes it, an ordered list of parts
//! left to right looking along the road's forward direction, and the one check that turns it
//! into the cross-section an edge stores (forward lanes, backward lanes, median) or refuses it
//! with a stable key. The built-in types the Roads menu offers are one table here; the player's
//! own types live in the per-user library file ([`library`]).
//!
//! A type also sets a road's attributes beyond its parts ([`RoadAttributes`]): its speed, whether
//! it has sidewalks (none or both: the layout says) and whether buildings zone along it.
//!
//! A type is tool configuration, not simulation state: a built road keeps its own lanes, median
//! and attributes in its edge, so no save, undo record or simulation system names a type, and
//! editing a type changes only the roads drawn after it. Engine-free: the Godot API resolves the
//! file path and converts the parts (`nodes::simulation_node::network_api::road_types`).

pub(crate) mod cost;
pub(crate) mod library;
#[cfg(test)]
mod tests;

use super::graph::{Edge, MAX_LANES_PER_DIRECTION, MedianStyle, RoadMedian};
use super::roadbed::{FOOTPATH_MIN_WIDTH_M, SHOULDER_BAND_WIDTH_M};
use super::types::TransitFlags;
use crate::config;

/// One part of a road layout, as the road builder shows it from the side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RoadPart {
    /// A sidewalk with its kerb, `config::SIDEWALK_WIDTH` wide; a car road has one at each end
    /// or none.
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

/// What a valid layout builds: the lane counts, median and sidewalks an edge stores. A car road
/// carries a sidewalk on each side or none (a 0.75 m shoulder each side instead); a walkway
/// (0 + 0 lanes) is one footpath band.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoadCrossSection {
    /// Lanes along the road's forward direction.
    pub fwd_lanes: u8,
    /// Lanes against it.
    pub bkw_lanes: u8,
    /// The median between the two directions, if any.
    pub median: Option<RoadMedian>,
    /// Whether a car road has its two sidewalks (the edge's `FOOT` flag); false for a walkway,
    /// whose footpath is not a sidewalk.
    pub sidewalks: bool,
}

impl RoadCrossSection {
    /// A walkway: no car lane, no median, no sidewalk.
    pub const WALKWAY: Self = Self {
        fwd_lanes: 0,
        bkw_lanes: 0,
        median: None,
        sidewalks: false,
    };

    // A car road with sidewalks and no median.
    const fn undivided(fwd_lanes: u8, bkw_lanes: u8) -> Self {
        Self {
            fwd_lanes,
            bkw_lanes,
            median: None,
            sidewalks: true,
        }
    }

    /// The cross-section `edge` carries: its lanes and median, and on a car road its `FOOT`
    /// flag as the sidewalks. `O(1)`.
    pub fn of_edge(edge: &Edge) -> Self {
        let walkway = edge.fwd_lanes == 0 && edge.bkw_lanes == 0;
        Self {
            fwd_lanes: edge.fwd_lanes,
            bkw_lanes: edge.bkw_lanes,
            median: edge.median,
            sidewalks: !walkway && edge.allowed_types & TransitFlags::FOOT != 0,
        }
    }

    /// Whether this is a walkway (no car lane).
    pub fn is_walkway(self) -> bool {
        self.fwd_lanes == 0 && self.bkw_lanes == 0
    }

    /// The layout this cross-section is drawn from, left to right along the forward direction:
    /// a walkway's footpath, else a sidewalk (when it has them), the backward lanes, the median,
    /// the forward lanes and a sidewalk (right-hand traffic; mirrored when driving on the left).
    /// `O(lanes)`.
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
        let sidewalk = self.sidewalks.then_some(RoadPart::Sidewalk);
        let mut parts = Vec::with_capacity(usize::from(left + right) + 3);
        parts.extend(sidewalk);
        parts.extend((0..left).map(|_| left_lane));
        parts.extend(self.median.map(RoadPart::Median));
        parts.extend((0..right).map(|_| right_lane));
        parts.extend(sidewalk);
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
/// - a sidewalk at one end only, or one inside the road: `layout_sidewalks` (a car road has a
///   sidewalk at both ends or none);
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
    let ends = [parts.first(), parts.last()].map(|end| end == Some(&RoadPart::Sidewalk));
    let sidewalks = ends == [true, true];
    if ends[0] != ends[1] || (sidewalks && parts.len() < 2) {
        return Err(LayoutRefusal::new("layout_sidewalks", None));
    }
    let skip = usize::from(sidewalks);
    let inside = &parts[skip..parts.len() - skip];
    // Lanes counted left and right of the centre line, and how many of each the median follows.
    let (mut left, mut right) = (0usize, 0usize);
    let mut median: Option<(usize, RoadMedian, usize, usize)> = None;
    for (offset, part) in inside.iter().enumerate() {
        let at = Some(offset + skip);
        match *part {
            RoadPart::Sidewalk => return Err(LayoutRefusal::new("layout_sidewalks", at)),
            RoadPart::Median(found) => {
                if median.is_some() {
                    return Err(LayoutRefusal::new("layout_two_medians", at));
                }
                median = Some((offset + skip, found, left, right));
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
        sidewalks,
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
/// the edge's stored width for a valid layout) and the total footprint the roadbed builds,
/// sidewalks included, or for a car road without sidewalks its two 0.75 m shoulders. Defined
/// for refused layouts too, so the builder can draw them. `O(parts)`.
pub fn layout_widths(parts: &[RoadPart]) -> (Vec<f32>, f32, f32) {
    let widths: Vec<f32> = parts.iter().map(|part| part.width_m()).collect();
    let total: f32 = widths.iter().sum();
    let sidewalks: f32 = parts
        .iter()
        .filter(|part| **part == RoadPart::Sidewalk)
        .map(|part| part.width_m())
        .sum();
    let shoulders = parts
        .iter()
        .all(|part| matches!(part, RoadPart::CarLane { .. } | RoadPart::Median(_)))
        && !parts.is_empty();
    let shoulders_m = if shoulders {
        2.0 * SHOULDER_BAND_WIDTH_M
    } else {
        0.0
    };
    (widths, total - sidewalks, total + shoulders_m)
}

/// The slowest speed a road type may set, in km/h.
pub const MIN_SPEED_KMH: u16 = 20;
/// The fastest speed a road type may set, in km/h.
pub const MAX_SPEED_KMH: u16 = 120;
/// The step between a road type's speeds, in km/h.
pub const SPEED_STEP_KMH: u16 = 10;
/// The speed of every built-in car road, in km/h (`config::DEFAULT_URBAN_ROAD_SPEED_MS`).
pub const DEFAULT_SPEED_KMH: u16 = 50;

/// A road type's speed for `section` in km/h, or `speed_out_of_range` outside
/// [`MIN_SPEED_KMH`]..=[`MAX_SPEED_KMH`] or off the [`SPEED_STEP_KMH`] grid. A walkway has no
/// speed: always 0, whatever is asked. `O(1)`.
pub fn check_speed_kmh(section: RoadCrossSection, speed_kmh: i64) -> Result<u16, &'static str> {
    if section.is_walkway() {
        return Ok(0);
    }
    u16::try_from(speed_kmh)
        .ok()
        .filter(|speed| {
            (MIN_SPEED_KMH..=MAX_SPEED_KMH).contains(speed) && speed % SPEED_STEP_KMH == 0
        })
        .ok_or("speed_out_of_range")
}

/// What a road type sets on a stroke's edge beyond its lanes and median (`ROAD-101`): the speed
/// limit, whether it has sidewalks (the edge's `FOOT` flag on a car road) and whether buildings
/// zone along it (the edge's `no_building_spawn` cleared). The road tool carries one beside its
/// median through every hover check, preview, commit and refusal report.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoadAttributes {
    /// The edge's speed limit in m/s.
    pub speed_ms: f32,
    /// Whether a car road has its two sidewalks; a walkway is walked whatever this says.
    pub sidewalks: bool,
    /// Whether lots and buildings zone along the road.
    pub zoning: bool,
}

impl RoadAttributes {
    /// Every built-in type's: 50 km/h, sidewalks, zoning on (bitwise the edge every stroke
    /// built before `ROAD-101`).
    pub const DEFAULT: Self = Self {
        speed_ms: config::DEFAULT_URBAN_ROAD_SPEED_MS,
        sidewalks: true,
        zoning: true,
    };

    /// The attributes of a type at `speed_kmh` (a walkway's 0 keeps the default limit, which
    /// walking ignores), with or without sidewalks and zoning. `f32::from(50) * KMH_TO_MPS` is
    /// bitwise [`config::DEFAULT_URBAN_ROAD_SPEED_MS`].
    pub fn new(speed_kmh: u16, sidewalks: bool, zoning: bool) -> Self {
        let speed_ms = if speed_kmh == 0 {
            config::DEFAULT_URBAN_ROAD_SPEED_MS
        } else {
            f32::from(speed_kmh) * config::KMH_TO_MPS
        };
        Self {
            speed_ms,
            sidewalks,
            zoning,
        }
    }

    /// The road tool's attributes from Godot's integers, or `speed_out_of_range` (see
    /// [`check_speed_kmh`]; the car-road range applies, as the tool cannot know a walkway).
    pub fn from_tool(speed_kmh: i64, sidewalks: bool, zoning: bool) -> Result<Self, &'static str> {
        let car_road = RoadCrossSection::undivided(1, 0);
        check_speed_kmh(car_road, speed_kmh).map(|speed| Self::new(speed, sidewalks, zoning))
    }

    /// How far a stroke's parcel corridor reaches beyond each side of its carriageway: a
    /// sidewalk's width, or the 0.75 m shoulder of a car road without sidewalks (a walkway keeps
    /// the sidewalk's margin, as before road types). `O(1)`.
    pub fn corridor_side_m(self, walkway: bool) -> f32 {
        if walkway || self.sidewalks {
            config::SIDEWALK_WIDTH
        } else {
            SHOULDER_BAND_WIDTH_M
        }
    }

    /// The attributes `edge` carries: its speed limit, `FOOT` (sidewalks on a car road) and
    /// `no_building_spawn` cleared (zoning). `O(1)`.
    pub fn of_edge(edge: &Edge) -> Self {
        Self {
            speed_ms: edge.speed_limit,
            sidewalks: edge.allowed_types & TransitFlags::FOOT != 0,
            zoning: !edge.no_building_spawn,
        }
    }

    /// Whether `self` and `other` set the same edge, the speed compared bitwise. `O(1)`.
    pub fn same_as(self, other: Self) -> bool {
        self.speed_ms.to_bits() == other.speed_ms.to_bits()
            && self.sidewalks == other.sidewalks
            && self.zoning == other.zoning
    }

    /// The speed limit in whole km/h (an edge's `speed_limit` read back). `O(1)`.
    pub fn speed_kmh_of(speed_ms: f32) -> u16 {
        (speed_ms / config::KMH_TO_MPS).round().clamp(0.0, f32::from(u16::MAX)) as u16
    }
}

/// A type the Roads menu offers in every game, in its button order.
#[derive(Clone, Copy, Debug)]
pub struct BuiltinRoadType {
    /// Stable id (`builtin:...`), never stored in a save.
    pub id: &'static str,
    /// The menu's label.
    pub name: &'static str,
    /// What it builds (every built-in car road has sidewalks, zones and runs at
    /// [`DEFAULT_SPEED_KMH`]).
    pub cross_section: RoadCrossSection,
}

/// The built-in road types, in the Roads menu's order: Walkway, 2-Lane Road (1 + 1), 4-Lane
/// Road (2 + 2), One-Way 1L (1 + 0), One-Way 2L (2 + 0). None has a median; the player's types
/// add those.
pub const BUILTIN_ROAD_TYPES: [BuiltinRoadType; 5] = [
    BuiltinRoadType {
        id: "builtin:walkway",
        name: "Walkway",
        cross_section: RoadCrossSection::WALKWAY,
    },
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
    /// Its roads' speed limit in km/h; 0 for a walkway, which has none.
    pub speed_kmh: u16,
    /// Whether lots and buildings zone along its roads.
    pub zoning: bool,
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
            speed_kmh: check_speed_kmh(builtin.cross_section, DEFAULT_SPEED_KMH.into())
                .expect("the default speed is in range"),
            zoning: true,
            verdict: Ok(builtin.cross_section),
        }
    }

    /// The cross-section, when the type is usable.
    pub fn usable(&self) -> Option<RoadCrossSection> {
        self.verdict.as_ref().ok().copied()
    }
}

/// The first usable type in `types`' order whose cross-section (sidewalks included) is
/// `section` and whose speed is `speed_kmh` (a walkway's is not compared): what Road Properties
/// shows as a road's type (duplicate layouts resolve to the first). Zoning is not compared, so
/// a road's "No buildings" override keeps its type. `O(types)`.
pub fn type_of(types: &[RoadType], section: RoadCrossSection, speed_kmh: u16) -> Option<&RoadType> {
    types.iter().find(|ty| {
        ty.usable() == Some(section) && (section.is_walkway() || ty.speed_kmh == speed_kmh)
    })
}
