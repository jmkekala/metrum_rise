// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: lane_spec.rs
//  script_path: rust/src/simulation/network/graph/lane_spec.rs
//  module_name: lane_spec
//  version: 0.1.0
//  description: Per-lane identity for a road edge. Two lane counts cannot
//           express a bus lane, a turn pocket, or a varying width, so
//           each lane carries its own type, width, and turns.
//  kind: module
//  spec: docs/roads.md
//  internal_dependencies: [config, simulation/network/types.rs]
//  external_dependencies: [smallvec]
//  features: [lane-bands, parking-angle, turn-pockets, reversible-lanes]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-27
// ========================================================================

//! Per-lane identity for a road edge.
//!
//! An edge used to describe its lanes as two counts, `fwd_lanes` and
//! `bkw_lanes`, with every lane implicitly [`config::LANE_WIDTH`] wide and
//! carrying vehicles. That model cannot express a bus lane, a turn pocket, a
//! median, a cycle track, or a lane that is wider than its neighbors, and it
//! cannot express a lane that begins or ends partway along an edge, which is
//! what a merge, a diverge, and an on-ramp are made of.
//!
//! [`LaneSpec`] is one lane. [`LaneLayout`] is the ordered set of them an edge
//! carries. The stored counts are gone; `fwd_count` and `bkw_count` derive from
//! the layout, so the layout is the only place a lane is recorded.
//!
//! Ordering is the contract: lanes are stored from the leftmost backward lane
//! through to the rightmost forward lane, in the order they appear across the
//! carriageway. `roads.md` requires later medians, parking lanes, cycle tracks,
//! and tram reservations to be "explicit ordered bands instead of special-case
//! render offsets", and this is that ordering.

use crate::config;
use crate::simulation::network::types::TransitFlags;
use smallvec::SmallVec;

// ========================================================================
// LANE KINDS
// ========================================================================

/// Lanes held inline before the layout spills to the heap.
///
/// The road tool caps each direction at four, so four plus four plus a median
/// is the widest road that can currently be authored, and two is the common
/// case. Four inline covers an ordinary road and a dual carriageway without
/// allocating at all; a boulevard spills, which is rare and cheap.
pub type LaneVec = SmallVec<[LaneSpec; 4]>;

/// What a lane is for. Wider than [`TransitFlags`] because a lane may be a
/// physical band that carries nothing at all, which is what a median is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LaneKind {
    /// Ordinary travel lane. Which modes may use it comes from `modes`.
    #[default]
    Travel,
    /// A physical separator. Carries no traffic and has no direction.
    ///
    /// One band covers the whole range from a painted hatch to a boulevard.
    /// At [`MEDIAN_MIN_WIDTH_M`] it is a line's worth of separation; widened,
    /// it becomes a flat no-turn centre, and wider still a planted boulevard.
    /// The kerbs, lines, and surface a median needs are the ones the roadbed
    /// already produces for its other bands, so widening reuses that machinery
    /// rather than adding geometry of its own.
    Median,
    /// Curbside parking. The angle it is marked at decides how wide the band
    /// is and how many cars fit per meter of curb, so it is carried on the
    /// lane rather than inferred from width.
    Parking,
    /// A planted strip between the carriageway and the sidewalk, or a planter
    /// dotted along it.
    ///
    /// Carries no traffic and takes width, like a median, but sits outboard of
    /// the carriageway rather than between the directions. This is what makes
    /// a street tree-lined, and what separates a wide sidewalk from a wide
    /// road with a narrow sidewalk on it.
    Verge,
    /// A hard shoulder or breakdown lane.
    Shoulder,
    /// A cycle track: part of the carriageway, not the footway.
    ///
    /// Whether it is painted or physically separated is [`LaneSpec::marking`]
    /// and width, not a different kind. A painted track is a `Dashed` or
    /// `Solid` boundary at [`CYCLE_TRACK_WIDTH_M`]; a protected one is the
    /// same band with a [`LaneKind::Median`] beside it, which is exactly how
    /// it is built in the world.
    CycleTrack,
    /// A centre lane whose direction is not fixed.
    ///
    /// Two real arrangements share this band, and they differ only by whether
    /// the direction changes on a schedule or per vehicle:
    ///
    /// - A **two-way left-turn lane**, the continuous centre lane an American
    ///   arterial carries, which either direction may enter to wait for a gap.
    /// - A **tidal lane**, which runs one way during the morning peak and the
    ///   other during the evening.
    ///
    /// Both are why a three-lane road exists at all, and neither can be
    /// expressed by a forward count and a backward count, because the lane
    /// belongs to neither total.
    Reversible,
}

/// Narrowest a median may be: a painted separation with no physical width to
/// speak of.
pub const MEDIAN_MIN_WIDTH_M: f32 = 0.1;

/// Above this, a median reads as a boulevard rather than a separator, and is
/// wide enough to plant, to hold a pedestrian refuge, or to carry a tram
/// reservation later.
pub const MEDIAN_BOULEVARD_WIDTH_M: f32 = 4.0;

/// Width of a cycle track. Narrower than a traffic lane because a bicycle is,
/// and because taking a full lane's width is what makes cities refuse to build
/// them.
pub const CYCLE_TRACK_WIDTH_M: f32 = 2.0;

// ========================================================================
// PARKING GEOMETRY
// ========================================================================

/// Width of a parallel curbside parking lane, which is a car width across
/// rather than a driving lane's clearance.
pub const PARKING_WIDTH_M: f32 = 2.5;

/// Depth a car occupies when parked at 45 degrees to the curb.
pub const PARKING_45_DEPTH_M: f32 = 4.8;

/// Depth a car occupies when parked square to the curb.
pub const PARKING_90_DEPTH_M: f32 = 5.5;

/// Curb length one car consumes when parked parallel, including the gap
/// needed to get in and out of the space.
pub const PARKING_PARALLEL_PITCH_M: f32 = 6.0;

/// Curb length one car consumes at 45 degrees. Angling trades roadway depth
/// for curb frontage, which is the whole reason a street is marked this way.
pub const PARKING_45_PITCH_M: f32 = 3.5;

/// Curb length one car consumes at 90 degrees.
pub const PARKING_90_PITCH_M: f32 = 2.7;

/// Default width of a planted verge between carriageway and sidewalk.
pub const VERGE_WIDTH_M: f32 = 1.8;

/// How a parking lane is marked, which sets both how deep the band is and how
/// many cars fit along a meter of curb.
///
/// Angling is a real tradeoff rather than a decoration: ninety degree parking
/// fits more than twice the cars of parallel along the same curb and takes
/// more than twice the roadway depth to do it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ParkingAngle {
    /// Parallel to the curb.
    #[default]
    Parallel,
    /// Angled at forty-five degrees, the common compromise.
    Angled45,
    /// Square to the curb, sometimes called perpendicular.
    Perpendicular90,
}

impl ParkingAngle {
    /// How deep the parking band is, in meters.
    pub fn depth_m(self) -> f32 {
        match self {
            Self::Parallel => PARKING_WIDTH_M,
            Self::Angled45 => PARKING_45_DEPTH_M,
            Self::Perpendicular90 => PARKING_90_DEPTH_M,
        }
    }

    /// Curb length one car consumes, in meters.
    pub fn pitch_m(self) -> f32 {
        match self {
            Self::Parallel => PARKING_PARALLEL_PITCH_M,
            Self::Angled45 => PARKING_45_PITCH_M,
            Self::Perpendicular90 => PARKING_90_PITCH_M,
        }
    }

    /// Spaces this angle yields along `curb_m` meters of curb.
    pub fn spaces_along(self, curb_m: f32) -> u32 {
        if curb_m <= 0.0 {
            return 0;
        }
        (curb_m / self.pitch_m()).floor().max(0.0) as u32
    }

    /// A stable identifier for save files and the Godot boundary.
    pub fn as_ordinal(self) -> u8 {
        match self {
            Self::Parallel => 0,
            Self::Angled45 => 1,
            Self::Perpendicular90 => 2,
        }
    }

    /// Recover an angle from its ordinal, defaulting to parallel.
    pub fn from_ordinal(value: u8) -> Self {
        match value {
            1 => Self::Angled45,
            2 => Self::Perpendicular90,
            _ => Self::Parallel,
        }
    }
}

// ========================================================================
// DESCRIPTORS
// ========================================================================

/// Which way a lane runs relative to the edge's own geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LaneDirection {
    /// Start node to end node.
    #[default]
    Forward,
    /// End node to start node.
    Backward,
    /// Neither: a median or a verge.
    None,
}

/// The marking painted on a lane's left boundary, looking along the lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LaneMarking {
    /// No painted line, which is the edge of the carriageway.
    #[default]
    None,
    /// Dashed: crossing permitted.
    Dashed,
    /// Solid: crossing discouraged or forbidden.
    Solid,
    /// Double solid: opposing traffic, crossing forbidden.
    DoubleSolid,
}

/// Which turns a lane may take at the far node. An empty set means every
/// non-U-turn movement is permitted, which is the current behaviour for a node
/// with no authored connections.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct TurnSet(pub u8);

impl TurnSet {
    /// Movement bit: a left turn at the far node.
    pub const LEFT: u8 = 1 << 0;
    /// Movement bit: straight on through the far node.
    pub const THROUGH: u8 = 1 << 1;
    /// Movement bit: a right turn at the far node.
    pub const RIGHT: u8 = 1 << 2;
    /// Movement bit: a U-turn, which ordinary lanes exclude by default.
    pub const U_TURN: u8 = 1 << 3;

    /// No restriction declared: every ordinary movement is allowed.
    pub const ANY: TurnSet = TurnSet(0);

    /// True when no restriction is declared, so every ordinary movement is
    /// permitted.
    #[inline]
    pub fn is_unrestricted(self) -> bool {
        self.0 == 0
    }

    /// True when `movement` is permitted from this lane.
    #[inline]
    pub fn allows(self, movement: u8) -> bool {
        self.is_unrestricted() || (self.0 & movement) != 0
    }
}

impl TurnSet {
    /// Classify a movement from the signed turn angle in radians, positive
    /// for a left turn in the edge's own frame.
    ///
    /// The bands are deliberately wide: anything inside a right angle either
    /// side of straight is a through movement, because a gently curving road
    /// is not a turn and should not need a turn set to say so. Beyond that it
    /// is a left or a right, and a reversal is a U-turn.
    pub fn movement_for_angle(angle_rad: f32) -> u8 {
        const QUARTER: f32 = std::f32::consts::FRAC_PI_4;
        const THREE_QUARTERS: f32 = 3.0 * std::f32::consts::FRAC_PI_4;
        let a = angle_rad;
        if a.abs() <= QUARTER {
            Self::THROUGH
        } else if a.abs() >= THREE_QUARTERS {
            Self::U_TURN
        } else if a > 0.0 {
            Self::LEFT
        } else {
            Self::RIGHT
        }
    }

    /// True when a lane with this turn set may make the movement implied by
    /// `angle_rad`.
    pub fn allows_angle(self, angle_rad: f32) -> bool {
        self.allows(Self::movement_for_angle(angle_rad))
    }
}

/// The longitudinal extent of a lane along its edge, as fractions of the
/// edge's length.
///
/// `start` 0.0 and `end` 1.0 is a lane running the whole edge, which is what
/// every lane does today. A lane that begins or ends inside those bounds is a
/// merge, a diverge, an auxiliary lane between ramps, or a turn pocket, and it
/// is the reason this type exists.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LaneRange {
    /// Where the lane begins, as a fraction of edge length.
    pub start: f32,
    /// Where the lane ends, as a fraction of edge length.
    pub end: f32,
}

impl Default for LaneRange {
    fn default() -> Self {
        Self {
            start: 0.0,
            end: 1.0,
        }
    }
}

impl LaneRange {
    /// A lane running the whole edge, which is what every lane does today.
    #[inline]
    pub fn full() -> Self {
        Self::default()
    }

    /// True when the lane spans its whole edge, which lets the geometry
    /// builder take the cheap path.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.start <= 0.0 && self.end >= 1.0
    }

    /// True when the lane is live at `t` along the edge.
    #[inline]
    pub fn contains(&self, t: f32) -> bool {
        t >= self.start && t <= self.end
    }
}

// ========================================================================
// ONE LANE
// ========================================================================

/// One lane on one edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LaneSpec {
    /// What the lane is for.
    pub kind: LaneKind,
    /// Which way it runs relative to the edge geometry.
    pub direction: LaneDirection,
    /// Width in metres. Defaults to [`config::LANE_WIDTH`], and varies for
    /// truck lanes, narrow urban lanes, cycle tracks, and medians.
    pub width_m: f32,
    /// Which modes may use this lane, from [`TransitFlags`].
    pub modes: u8,
    /// Marking on the left boundary.
    pub marking: LaneMarking,
    /// Permitted turns at the far node.
    pub turns: TurnSet,
    /// Longitudinal extent along the edge.
    pub range: LaneRange,
    /// How a parking band is marked. Meaningless on any other kind, and
    /// ignored there.
    pub parking_angle: ParkingAngle,
}

impl Default for LaneSpec {
    fn default() -> Self {
        Self {
            kind: LaneKind::Travel,
            direction: LaneDirection::Forward,
            width_m: config::LANE_WIDTH,
            modes: TransitFlags::ROAD_TRAFFIC,
            marking: LaneMarking::None,
            turns: TurnSet::ANY,
            range: LaneRange::full(),
            parking_angle: ParkingAngle::Parallel,
        }
    }
}

impl LaneSpec {
    /// An ordinary car lane of standard width running the whole edge.
    pub fn travel(direction: LaneDirection) -> Self {
        Self {
            direction,
            ..Self::default()
        }
    }

    /// A median of the given width, clamped to at least
    /// [`MEDIAN_MIN_WIDTH_M`]. It carries nothing and has no direction, so it
    /// never appears in a lane count and never receives a connector.
    pub fn median(width_m: f32) -> Self {
        Self {
            kind: LaneKind::Median,
            direction: LaneDirection::None,
            width_m: width_m.max(MEDIAN_MIN_WIDTH_M),
            modes: 0,
            marking: LaneMarking::Solid,
            turns: TurnSet::ANY,
            range: LaneRange::full(),
            parking_angle: ParkingAngle::Parallel,
        }
    }

    /// The slimmest median: separation without width, which is a painted
    /// centre rather than a built one.
    pub fn median_painted() -> Self {
        Self::median(MEDIAN_MIN_WIDTH_M)
    }

    /// A bus lane: an ordinary travel band that private cars may not use.
    ///
    /// Nothing about its geometry differs from a travel lane, which is the
    /// point. It is defined by the mode bit it withholds, so the surface
    /// builder, the pathfinder, and the renderer all treat it as the lane it
    /// physically is while routing refuses to put a car on it.
    pub fn bus(direction: LaneDirection) -> Self {
        Self {
            modes: TransitFlags::BUS,
            marking: LaneMarking::Solid,
            ..Self::travel(direction)
        }
    }

    /// A part-time bus lane: buses only during the hours it is in force, and
    /// ordinary traffic outside them.
    ///
    /// Stored as both bits with the restriction expressed by the schedule the
    /// caller applies, because a lane that is sometimes open to cars must be
    /// build as a lane cars can be on.
    pub fn bus_part_time(direction: LaneDirection) -> Self {
        Self {
            modes: TransitFlags::ROAD_TRAFFIC,
            marking: LaneMarking::Dashed,
            ..Self::travel(direction)
        }
    }

    /// A cycle track of the standard width, carrying bicycles alone.
    pub fn cycle_track(direction: LaneDirection) -> Self {
        Self {
            kind: LaneKind::CycleTrack,
            direction,
            width_m: CYCLE_TRACK_WIDTH_M,
            modes: TransitFlags::BIKE,
            marking: LaneMarking::Solid,
            turns: TurnSet::ANY,
            range: LaneRange::full(),
            parking_angle: ParkingAngle::Parallel,
        }
    }

    /// Kerbside parking, which carries no moving traffic and takes width.
    pub fn parking() -> Self {
        Self::parking_at(ParkingAngle::Parallel)
    }

    /// Curbside parking marked at `angle`.
    ///
    /// The angle sets the band's depth, so a street that switches from
    /// parallel to ninety degree parking gets wider by the difference and the
    /// carriageway does not move.
    pub fn parking_at(angle: ParkingAngle) -> Self {
        Self {
            kind: LaneKind::Parking,
            direction: LaneDirection::None,
            width_m: angle.depth_m(),
            modes: 0,
            marking: LaneMarking::Dashed,
            turns: TurnSet::ANY,
            range: LaneRange::full(),
            parking_angle: angle,
        }
    }

    /// A planted verge or a run of planters between road and sidewalk.
    pub fn verge(width_m: f32) -> Self {
        Self {
            kind: LaneKind::Verge,
            direction: LaneDirection::None,
            width_m: width_m.max(0.0),
            modes: 0,
            marking: LaneMarking::None,
            turns: TurnSet::ANY,
            range: LaneRange::full(),
            parking_angle: ParkingAngle::Parallel,
        }
    }

    /// How many cars this lane holds along `curb_m` meters of curb, and zero
    /// for any band that is not parking.
    pub fn parking_spaces_along(&self, curb_m: f32) -> u32 {
        if self.kind != LaneKind::Parking {
            return 0;
        }
        self.parking_angle.spaces_along(curb_m)
    }

    /// A turn pocket: a travel lane that exists only over the last `fraction`
    /// of the edge, restricted to the movements in `turns`.
    ///
    /// This is what [`LaneRange`] was added for. The lane is absent at the
    /// start of the edge and live at the node, so it widens the carriageway
    /// where the queue forms and nowhere else.
    pub fn turn_pocket(direction: LaneDirection, turns: TurnSet, fraction: f32) -> Self {
        let start = (1.0 - fraction.clamp(0.0, 1.0)).clamp(0.0, 1.0);
        Self {
            turns,
            range: LaneRange { start, end: 1.0 },
            ..Self::travel(direction)
        }
    }

    /// A two-way left-turn lane: the continuous centre band either direction
    /// may enter to wait for a gap.
    ///
    /// It has no direction of its own, which is the point. A vehicle from
    /// either side occupies it briefly and leaves across the opposing flow, so
    /// it belongs to neither lane count and cannot be expressed by them.
    pub fn two_way_left_turn() -> Self {
        Self {
            kind: LaneKind::Reversible,
            direction: LaneDirection::None,
            width_m: config::LANE_WIDTH,
            modes: TransitFlags::ROAD_TRAFFIC,
            // Dashed on the inside, solid on the outside, is what the road is
            // actually painted with; the marking here is the crossable one
            // because entering the lane is legal from both sides.
            marking: LaneMarking::Dashed,
            turns: TurnSet(TurnSet::LEFT),
            range: LaneRange::full(),
            parking_angle: ParkingAngle::Parallel,
        }
    }

    /// A tidal lane: a centre band that carries the peak direction.
    ///
    /// `direction` is which way it runs right now. Flipping it is what a
    /// reversible-lane policy does, and the geometry does not change when it
    /// happens, because the band is where it always was.
    pub fn tidal(direction: LaneDirection) -> Self {
        Self {
            kind: LaneKind::Reversible,
            direction,
            width_m: config::LANE_WIDTH,
            modes: TransitFlags::ROAD_TRAFFIC,
            marking: LaneMarking::Dashed,
            turns: TurnSet::ANY,
            range: LaneRange::full(),
            parking_angle: ParkingAngle::Parallel,
        }
    }

    /// True when this lane's direction may change, either per vehicle or on a
    /// schedule, so a router must not cache it as fixed.
    #[inline]
    pub fn is_reversible(&self) -> bool {
        matches!(self.kind, LaneKind::Reversible)
    }

    /// Restrict this lane to the given modes, which is how a street is
    /// pedestrianised without becoming a different kind of object.
    ///
    /// The road keeps its geometry, its address, its deliveries, and its
    /// frontages. Only who may drive on it changes.
    pub fn restricted_to(mut self, modes: u8) -> Self {
        self.modes = modes;
        self
    }

    /// True when this lane carries moving traffic, which excludes medians,
    /// parking, and shoulders.
    #[inline]
    pub fn is_travel(&self) -> bool {
        matches!(self.kind, LaneKind::Travel)
    }

    /// True when this median is wide enough to read as a boulevard, which is
    /// what decides whether it is planted and kerbed rather than painted.
    #[inline]
    pub fn is_boulevard(&self) -> bool {
        matches!(self.kind, LaneKind::Median) && self.width_m >= MEDIAN_BOULEVARD_WIDTH_M
    }

    /// True when a vehicle may cross this band to turn across oncoming
    /// traffic. A painted median may be crossed; a built one may not, which is
    /// what makes a boulevard a no-turn centre.
    #[inline]
    pub fn blocks_turns_across(&self) -> bool {
        matches!(self.kind, LaneKind::Median) && self.width_m > MEDIAN_MIN_WIDTH_M
    }

    /// True when this lane carries moving traffic of any kind, which excludes
    /// medians, parking, and shoulders but includes cycle tracks.
    ///
    /// Distinct from [`LaneSpec::is_travel`], which asks whether this is a
    /// general-purpose traffic lane. A cycle track is not one of those and
    /// still carries bicycles, so the two questions have different answers and
    /// conflating them makes every cycle track admit nothing at all.
    #[inline]
    pub fn is_moving(&self) -> bool {
        matches!(
            self.kind,
            LaneKind::Travel | LaneKind::CycleTrack | LaneKind::Reversible
        )
    }

    /// True when this lane carries `mode`.
    ///
    /// A band that carries no moving traffic carries no mode either, whatever
    /// its mode bits say, which is what keeps a median out of every count.
    #[inline]
    pub fn carries(&self, mode: u8) -> bool {
        self.is_moving() && (self.modes & mode) != 0
    }
}

// ========================================================================
// THE LAYOUT
// ========================================================================

/// The ordered set of lanes an edge carries, left to right across the
/// carriageway.
///
/// "Left to right" is in the edge's own frame: backward lanes first, outermost
/// first, then forward lanes, innermost first. That ordering is what lets a
/// median sit between the two directions as an ordinary entry rather than as a
/// special case.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LaneLayout {
    lanes: LaneVec,
    /// Sidewalk width per side, in meters, or `None` to use the project
    /// default.
    ///
    /// Authored rather than fixed because a wide sidewalk is a design
    /// decision: a high street, a promenade, and a residential lane want
    /// different widths, and holding it here keeps it beside the bands it
    /// sits outboard of.
    sidewalk_width_m: Option<f32>,
}

impl LaneLayout {
    /// The layout an edge with these counts would have had under the old
    /// two-integer model. Every lane is standard width and carries cars.
    ///
    /// This is the migration path: an edge built from counts produces exactly
    /// the geometry it produced before.
    pub fn from_counts(fwd: u8, bkw: u8) -> Self {
        let mut lanes = LaneVec::with_capacity(usize::from(fwd) + usize::from(bkw));
        for _ in 0..bkw {
            lanes.push(LaneSpec::travel(LaneDirection::Backward));
        }
        for _ in 0..fwd {
            lanes.push(LaneSpec::travel(LaneDirection::Forward));
        }
        Self {
            lanes,
            sidewalk_width_m: None,
        }
    }

    /// A layout from an explicit ordered list of lanes.
    /// Build a layout from the flat integer form the Godot boundary speaks.
    ///
    /// Each band is seven values in order: kind, direction, width in
    /// millimeters, mode bits, marking, turn set, and parking angle. Widths
    /// travel as integers because the boundary carries an `i32` array and a
    /// millimeter is finer than any road is authored to.
    ///
    /// A malformed run returns `None` rather than a partial road, because half
    /// a cross-section is worse than none.
    pub fn from_flat(values: &[i32]) -> Option<Self> {
        const STRIDE: usize = 7;
        if values.is_empty() || values.len() % STRIDE != 0 {
            return None;
        }
        let mut lanes = LaneVec::with_capacity(values.len() / STRIDE);
        for band in values.chunks_exact(STRIDE) {
            let kind = match band[0] {
                0 => LaneKind::Travel,
                1 => LaneKind::Median,
                2 => LaneKind::Parking,
                3 => LaneKind::Shoulder,
                4 => LaneKind::CycleTrack,
                5 => LaneKind::Reversible,
                6 => LaneKind::Verge,
                _ => return None,
            };
            let direction = match band[1] {
                0 => LaneDirection::Forward,
                1 => LaneDirection::Backward,
                2 => LaneDirection::None,
                _ => return None,
            };
            let marking = match band[4] {
                0 => LaneMarking::None,
                1 => LaneMarking::Dashed,
                2 => LaneMarking::Solid,
                3 => LaneMarking::DoubleSolid,
                _ => return None,
            };
            if !(0..=2).contains(&band[6]) {
                return None;
            }
            let width_m = (band[2] as f32) / 1000.0;
            if width_m <= 0.0 {
                return None;
            }
            lanes.push(LaneSpec {
                kind,
                direction,
                width_m,
                modes: band[3].clamp(0, i32::from(u8::MAX)) as u8,
                marking,
                turns: TurnSet(band[5].clamp(0, i32::from(u8::MAX)) as u8),
                range: LaneRange::full(),
                parking_angle: ParkingAngle::from_ordinal(band[6] as u8),
            });
        }
        Some(Self {
            lanes,
            sidewalk_width_m: None,
        })
    }

    /// The flat integer form of this layout, for the Godot boundary.
    ///
    /// Inverse of [`LaneLayout::from_flat`], so a cross-section can be read
    /// into an editor and written back without passing through a count.
    pub fn to_flat(&self) -> Vec<i32> {
        let mut out = Vec::with_capacity(self.lanes.len() * 7);
        for lane in self.lanes.iter() {
            out.push(match lane.kind {
                LaneKind::Travel => 0,
                LaneKind::Median => 1,
                LaneKind::Parking => 2,
                LaneKind::Shoulder => 3,
                LaneKind::CycleTrack => 4,
                LaneKind::Reversible => 5,
                LaneKind::Verge => 6,
            });
            out.push(match lane.direction {
                LaneDirection::Forward => 0,
                LaneDirection::Backward => 1,
                LaneDirection::None => 2,
            });
            out.push((lane.width_m * 1000.0).round() as i32);
            out.push(i32::from(lane.modes));
            out.push(match lane.marking {
                LaneMarking::None => 0,
                LaneMarking::Dashed => 1,
                LaneMarking::Solid => 2,
                LaneMarking::DoubleSolid => 3,
            });
            out.push(i32::from(lane.turns.0));
            out.push(i32::from(lane.parking_angle.as_ordinal()));
        }
        out
    }

    /// A layout from an explicit ordered list of lanes.
    pub fn from_lanes(lanes: impl IntoIterator<Item = LaneSpec>) -> Self {
        Self {
            lanes: lanes.into_iter().collect(),
            sidewalk_width_m: None,
        }
    }

    /// A divided road: `bkw` backward lanes, a median of `median_width_m`, then
    /// `fwd` forward lanes.
    ///
    /// The same call covers the whole family. At [`MEDIAN_MIN_WIDTH_M`] it is a
    /// painted centre line that vehicles may cross; widened it becomes a flat
    /// no-turn centre; at [`MEDIAN_BOULEVARD_WIDTH_M`] or more it is a
    /// boulevard. Nothing else about the edge changes, because the median is
    /// just another ordered band.
    pub fn divided(fwd: u8, bkw: u8, median_width_m: f32) -> Self {
        let mut lanes = LaneVec::with_capacity(usize::from(fwd) + usize::from(bkw) + 1);
        for _ in 0..bkw {
            lanes.push(LaneSpec::travel(LaneDirection::Backward));
        }
        lanes.push(LaneSpec::median(median_width_m));
        for _ in 0..fwd {
            lanes.push(LaneSpec::travel(LaneDirection::Forward));
        }
        Self {
            lanes,
            sidewalk_width_m: None,
        }
    }

    /// A road with parking on both kerbs, which is the ordinary urban street.
    pub fn with_parking(fwd: u8, bkw: u8) -> Self {
        let mut lanes = LaneVec::new();
        lanes.push(LaneSpec::parking());
        for _ in 0..bkw {
            lanes.push(LaneSpec::travel(LaneDirection::Backward));
        }
        for _ in 0..fwd {
            lanes.push(LaneSpec::travel(LaneDirection::Forward));
        }
        lanes.push(LaneSpec::parking());
        Self {
            lanes,
            sidewalk_width_m: None,
        }
    }

    /// A pedestrianised street: the same road, closed to private cars.
    ///
    /// Deliveries, emergency vehicles, and residents reaching their garages
    /// all still arrive, because the lanes are still lanes. Only the mode
    /// bits change, which is the difference between a pedestrian street and a
    /// footpath, and the reason modelling it as a path starves the businesses
    /// on it.
    pub fn pedestrianised(fwd: u8, bkw: u8, modes: u8) -> Self {
        let mut layout = Self::from_counts(fwd, bkw);
        for lane in layout.lanes.iter_mut() {
            if lane.is_travel() {
                lane.modes = modes;
            }
        }
        layout
    }

    /// The three-lane road: one lane each way with a shared centre band.
    ///
    /// The arrangement a two-way left-turn lane produces, and the reason an
    /// odd lane count is worth having rather than a rounding error. The centre
    /// belongs to neither direction, so `fwd_count` and `bkw_count` both
    /// return one and their sum is deliberately less than the lane total.
    pub fn with_two_way_left_turn(fwd: u8, bkw: u8) -> Self {
        let mut lanes = LaneVec::new();
        for _ in 0..bkw {
            lanes.push(LaneSpec::travel(LaneDirection::Backward));
        }
        lanes.push(LaneSpec::two_way_left_turn());
        for _ in 0..fwd {
            lanes.push(LaneSpec::travel(LaneDirection::Forward));
        }
        Self {
            lanes,
            sidewalk_width_m: None,
        }
    }

    /// A road whose centre lane currently runs `peak`, carrying the tidal
    /// flow, with fixed lanes either side.
    pub fn tidal(fwd: u8, bkw: u8, peak: LaneDirection) -> Self {
        let mut lanes = LaneVec::new();
        for _ in 0..bkw {
            lanes.push(LaneSpec::travel(LaneDirection::Backward));
        }
        lanes.push(LaneSpec::tidal(peak));
        for _ in 0..fwd {
            lanes.push(LaneSpec::travel(LaneDirection::Forward));
        }
        Self {
            lanes,
            sidewalk_width_m: None,
        }
    }

    /// Flip every reversible lane to `peak`.
    ///
    /// The geometry does not change, because the band is where it always was.
    /// Only which way it runs does, which is what makes this cheap enough to
    /// do twice a day.
    pub fn set_tidal_direction(&mut self, peak: LaneDirection) {
        for lane in self.lanes.iter_mut() {
            if lane.kind == LaneKind::Reversible && lane.direction != LaneDirection::None {
                lane.direction = peak;
            }
        }
    }

    /// True when any lane's direction may change.
    pub fn has_reversible(&self) -> bool {
        self.lanes.iter().any(|l| l.is_reversible())
    }

    /// Add turn pockets to the approach, which is how a road that carries two
    /// lanes between junctions arrives at one with four.
    ///
    /// This is the other reason a lane count cannot describe a road: the count
    /// is different at the two ends. A city street widens as it reaches the
    /// stop line, separating a left turn and a right turn out of the through
    /// traffic, and narrows again immediately after. Each pocket is a lane
    /// with a partial [`LaneRange`], so it takes width only where it exists.
    ///
    /// `fraction` is how much of the edge the pockets occupy, measured back
    /// from the far node.
    pub fn with_turn_pockets(
        mut self,
        left: u8,
        right: u8,
        direction: LaneDirection,
        fraction: f32,
    ) -> Self {
        // A left pocket sits inboard of the through lanes and a right pocket
        // outboard, which for a forward carriageway means the left goes before
        // the forward run and the right after it. Insert positions are found
        // before anything is added so the two do not interfere.
        let first_fwd = self
            .lanes
            .iter()
            .position(|l| l.direction == direction && l.is_travel());
        let insert_at = first_fwd.unwrap_or(self.lanes.len());
        for _ in 0..left {
            self.lanes.insert(
                insert_at,
                LaneSpec::turn_pocket(direction, TurnSet(TurnSet::LEFT), fraction),
            );
        }
        for _ in 0..right {
            self.lanes.push(LaneSpec::turn_pocket(
                direction,
                TurnSet(TurnSet::RIGHT),
                fraction,
            ));
        }
        self
    }

    /// Carriageway width at `t` along the edge, counting only the lanes live
    /// there.
    ///
    /// [`LaneLayout::asphalt_width`] is the widest the road ever gets, which is
    /// what the roadbed has to reserve. This is what it actually is at a given
    /// point, and the two differ exactly where a turn pocket opens.
    pub fn asphalt_width_at(&self, t: f32) -> f32 {
        self.lanes
            .iter()
            .filter(|l| l.range.contains(t))
            .map(|l| l.width_m)
            .sum()
    }

    /// The median band, if this layout has one.
    pub fn median(&self) -> Option<&LaneSpec> {
        self.lanes.iter().find(|l| l.kind == LaneKind::Median)
    }

    /// True when a vehicle may not turn across the centre of this road,
    /// because a built median stands in the way.
    pub fn is_no_turn_centre(&self) -> bool {
        self.median().is_some_and(|m| m.blocks_turns_across())
    }

    /// The lanes, ordered left to right across the carriageway.
    #[inline]
    pub fn lanes(&self) -> &[LaneSpec] {
        &self.lanes
    }

    /// Mutable access to the ordered lanes, for editing a layout in place.
    #[inline]
    pub fn lanes_mut(&mut self) -> &mut LaneVec {
        &mut self.lanes
    }

    /// True when this layout has spilled its inline storage onto the heap.
    ///
    /// Exposed so a test can assert that ordinary roads never allocate.
    #[inline]
    pub fn spilled(&self) -> bool {
        self.lanes.spilled()
    }

    /// True when the edge carries no lanes at all, which is a walkway.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.lanes.is_empty()
    }

    /// Number of forward travel lanes carrying cars, which is what
    /// `Edge::fwd_lanes` used to store.
    pub fn fwd_count(&self) -> u8 {
        self.count_travel(LaneDirection::Forward)
    }

    /// Number of backward travel lanes carrying cars.
    pub fn bkw_count(&self) -> u8 {
        self.count_travel(LaneDirection::Backward)
    }

    fn count_travel(&self, direction: LaneDirection) -> u8 {
        let n = self
            .lanes
            .iter()
            .filter(|l| l.direction == direction && l.carries(TransitFlags::CAR))
            .count();
        u8::try_from(n).unwrap_or(u8::MAX)
    }

    /// Total carriageway width in metres: every band, including medians,
    /// parking, and shoulders, but not sidewalks.
    ///
    /// Replaces `(fwd_lanes + bkw_lanes) as f32 * LANE_WIDTH`, and equals it
    /// exactly for a layout built by [`LaneLayout::from_counts`].
    pub fn asphalt_width(&self) -> f32 {
        self.lanes.iter().map(|l| l.width_m).sum()
    }

    /// Lateral offset of a lane's centreline from the carriageway centre, in
    /// metres, positive to the right in the edge's own frame.
    ///
    /// Under the old model this was `(index + 0.5) * LANE_WIDTH`, which only
    /// held because every lane was the same width. Here it accumulates real
    /// widths, so a wide truck lane pushes its neighbours outward correctly.
    pub fn centre_offset(&self, lane_index: usize) -> Option<f32> {
        if lane_index >= self.lanes.len() {
            return None;
        }
        let half = self.asphalt_width() * 0.5;
        let mut running = 0.0_f32;
        for lane in self.lanes.iter().take(lane_index) {
            running += lane.width_m;
        }
        Some(running + self.lanes[lane_index].width_m * 0.5 - half)
    }

    /// Every lane that is live at `t` along the edge, as indices into
    /// [`LaneLayout::lanes`]. A lane whose range excludes `t` has ended or has
    /// not begun, which is what a merge looks like from the middle.
    pub fn live_at(&self, t: f32) -> impl Iterator<Item = usize> + '_ {
        self.lanes
            .iter()
            .enumerate()
            .filter(move |(_, l)| l.range.contains(t))
            .map(|(i, _)| i)
    }

    /// True when every lane runs the whole edge, which is the common case and
    /// lets callers skip range handling entirely.
    pub fn all_full_length(&self) -> bool {
        self.lanes.iter().all(|l| l.range.is_full())
    }

    /// Number of lanes running `direction` that carry `mode`.
    ///
    /// This is what a router asks. `fwd_count` is this with
    /// [`TransitFlags::CAR`], and a bus asking the same question gets a
    /// different and larger answer on a road with a bus lane.
    pub fn count_for_mode(&self, direction: LaneDirection, mode: u8) -> u8 {
        let n = self
            .lanes
            .iter()
            .filter(|l| l.direction == direction && l.carries(mode))
            .count();
        u8::try_from(n).unwrap_or(u8::MAX)
    }

    /// True when any lane admits `mode` in either direction, which is what
    /// decides whether an edge is usable by a vehicle class at all.
    pub fn admits(&self, mode: u8) -> bool {
        self.lanes.iter().any(|l| l.carries(mode))
    }

    /// True when the carriageway admits no private cars, which is what makes a
    /// street pedestrianised rather than merely quiet.
    pub fn is_car_free(&self) -> bool {
        !self.admits(TransitFlags::CAR)
    }

    /// Indices of every lane carrying `mode`, in carriageway order.
    pub fn lanes_for_mode(&self, mode: u8) -> impl Iterator<Item = usize> + '_ {
        self.lanes
            .iter()
            .enumerate()
            .filter(move |(_, l)| l.carries(mode))
            .map(|(i, _)| i)
    }

    /// Total width of the parking bands, which is what a parking supply model
    /// counts and what a street loses when parking is removed.
    pub fn parking_width(&self) -> f32 {
        self.lanes
            .iter()
            .filter(|l| l.kind == LaneKind::Parking)
            .map(|l| l.width_m)
            .sum()
    }

    /// Total curbside spaces this cross-section yields over `curb_m` meters,
    /// summed across every parking band on both sides.
    ///
    /// This is the number a parking supply model reads, and it is why the
    /// angle is stored: the same width of street holds very different numbers
    /// of cars depending on how the bays are marked.
    pub fn parking_spaces_along(&self, curb_m: f32) -> u32 {
        self.lanes
            .iter()
            .map(|l| l.parking_spaces_along(curb_m))
            .sum()
    }

    /// Sidewalk width per side in meters, falling back to the project default
    /// when this layout does not author one.
    pub fn sidewalk_width(&self) -> f32 {
        self.sidewalk_width_m.unwrap_or(config::SIDEWALK_WIDTH)
    }

    /// The authored width, or `None` when this layout uses the default.
    ///
    /// Distinct from [`LaneLayout::sidewalk_width`] because a save has to
    /// record that a layout said nothing, so that changing the project default
    /// later moves every street that never overrode it.
    pub fn authored_sidewalk_width(&self) -> Option<f32> {
        self.sidewalk_width_m
    }

    /// Author a sidewalk width per side. `None` restores the default.
    pub fn set_sidewalk_width(&mut self, width_m: Option<f32>) {
        self.sidewalk_width_m = width_m.map(|w| w.max(0.0));
    }

    /// Builder form of [`LaneLayout::set_sidewalk_width`].
    pub fn with_sidewalk_width(mut self, width_m: f32) -> Self {
        self.set_sidewalk_width(Some(width_m));
        self
    }

    /// Total width of the planted verges, which is what separates a
    /// tree-lined street from a bare one of the same overall width.
    pub fn verge_width(&self) -> f32 {
        self.lanes
            .iter()
            .filter(|l| l.kind == LaneKind::Verge)
            .map(|l| l.width_m)
            .sum()
    }

    /// A street with parking at `angle` on both curbs, a planted verge outside
    /// each, and the given travel lanes between them.
    ///
    /// The ordinary residential arrangement, and the one that shows why these
    /// are bands: the carriageway is unchanged and the street is wider by
    /// exactly what the parking and the planting occupy.
    pub fn with_parking_and_verge(
        fwd: u8,
        bkw: u8,
        angle: ParkingAngle,
        verge_m: f32,
    ) -> Self {
        let mut lanes = LaneVec::new();
        lanes.push(LaneSpec::verge(verge_m));
        lanes.push(LaneSpec::parking_at(angle));
        for _ in 0..bkw {
            lanes.push(LaneSpec::travel(LaneDirection::Backward));
        }
        for _ in 0..fwd {
            lanes.push(LaneSpec::travel(LaneDirection::Forward));
        }
        lanes.push(LaneSpec::parking_at(angle));
        lanes.push(LaneSpec::verge(verge_m));
        Self {
            lanes,
            sidewalk_width_m: None,
        }
    }
}

// ========================================================================
// TESTS
// ========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_round_trip() {
        let layout = LaneLayout::from_counts(2, 3);
        assert_eq!(layout.fwd_count(), 2);
        assert_eq!(layout.bkw_count(), 3);
        assert_eq!(layout.lanes().len(), 5);
    }

    #[test]
    fn width_matches_the_old_formula() {
        for (fwd, bkw) in [(1u8, 1u8), (2, 2), (4, 0), (0, 3)] {
            let layout = LaneLayout::from_counts(fwd, bkw);
            let old = f32::from(fwd + bkw) * config::LANE_WIDTH;
            assert!(
                (layout.asphalt_width() - old).abs() < 1e-6,
                "width drifted for {fwd}/{bkw}: {} vs {old}",
                layout.asphalt_width()
            );
        }
    }

    #[test]
    fn offsets_match_the_old_formula() {
        // Old model: forward lane l sat at (l + 0.5) * LANE_WIDTH from centre,
        // backward lane l at the mirror of that. With a symmetric layout the
        // accumulated offsets must land on the same numbers.
        let layout = LaneLayout::from_counts(2, 2);
        let w = config::LANE_WIDTH;
        let expected = [-1.5 * w, -0.5 * w, 0.5 * w, 1.5 * w];
        for (i, want) in expected.iter().enumerate() {
            let got = layout.centre_offset(i).expect("lane exists");
            assert!(
                (got - want).abs() < 1e-6,
                "lane {i} offset {got} wanted {want}"
            );
        }
    }

    #[test]
    fn a_wide_lane_pushes_its_neighbours_out() {
        let mut layout = LaneLayout::from_counts(2, 0);
        layout.lanes_mut()[0].width_m = config::LANE_WIDTH * 2.0;
        let total = config::LANE_WIDTH * 3.0;
        assert!((layout.asphalt_width() - total).abs() < 1e-6);
        // The wide lane's centre sits one of its own half-widths from the left
        // kerb, not at the old fixed stride.
        let first = layout.centre_offset(0).expect("lane exists");
        assert!((first - (config::LANE_WIDTH - total * 0.5)).abs() < 1e-6);
    }

    #[test]
    fn a_median_carries_nothing_but_takes_width() {
        let mut lanes: Vec<LaneSpec> = vec![LaneSpec::travel(LaneDirection::Backward)];
        lanes.push(LaneSpec {
            kind: LaneKind::Median,
            direction: LaneDirection::None,
            width_m: 2.0,
            modes: 0,
            ..LaneSpec::default()
        });
        lanes.push(LaneSpec::travel(LaneDirection::Forward));
        let layout = LaneLayout::from_lanes(lanes);

        assert_eq!(layout.fwd_count(), 1);
        assert_eq!(layout.bkw_count(), 1);
        assert!((layout.asphalt_width() - (config::LANE_WIDTH * 2.0 + 2.0)).abs() < 1e-6);
        assert!(!layout.lanes()[1].carries(TransitFlags::CAR));
    }

    #[test]
    fn a_lane_that_ends_is_not_live_past_its_end() {
        let mut layout = LaneLayout::from_counts(2, 0);
        layout.lanes_mut()[1].range = LaneRange {
            start: 0.0,
            end: 0.5,
        };
        assert!(!layout.all_full_length());
        assert_eq!(layout.live_at(0.25).count(), 2);
        assert_eq!(layout.live_at(0.75).count(), 1);
    }

    #[test]
    fn a_painted_median_is_thin_and_crossable() {
        let layout = LaneLayout::divided(2, 2, MEDIAN_MIN_WIDTH_M);
        let median = layout.median().expect("divided road has a median");
        assert!((median.width_m - MEDIAN_MIN_WIDTH_M).abs() < 1e-6);
        assert!(!median.blocks_turns_across());
        assert!(!median.is_boulevard());
        assert!(!layout.is_no_turn_centre());
        // The lane counts ignore it entirely.
        assert_eq!(layout.fwd_count(), 2);
        assert_eq!(layout.bkw_count(), 2);
    }

    #[test]
    fn a_median_never_goes_below_the_floor() {
        let m = LaneSpec::median(0.0);
        assert!((m.width_m - MEDIAN_MIN_WIDTH_M).abs() < 1e-6);
        let m2 = LaneSpec::median(-5.0);
        assert!((m2.width_m - MEDIAN_MIN_WIDTH_M).abs() < 1e-6);
    }

    #[test]
    fn widening_a_median_makes_a_no_turn_centre_then_a_boulevard() {
        let narrow = LaneLayout::divided(2, 2, 1.5);
        assert!(narrow.is_no_turn_centre());
        assert!(!narrow.median().unwrap().is_boulevard());

        let boulevard = LaneLayout::divided(2, 2, MEDIAN_BOULEVARD_WIDTH_M);
        assert!(boulevard.is_no_turn_centre());
        assert!(boulevard.median().unwrap().is_boulevard());
    }

    #[test]
    fn a_median_widens_the_road_by_exactly_its_own_width() {
        let plain = LaneLayout::from_counts(2, 2);
        for w in [MEDIAN_MIN_WIDTH_M, 1.5, MEDIAN_BOULEVARD_WIDTH_M, 12.0] {
            let divided = LaneLayout::divided(2, 2, w);
            let grew = divided.asphalt_width() - plain.asphalt_width();
            assert!(
                (grew - w).abs() < 1e-6,
                "median of {w} m grew the road by {grew} m"
            );
        }
    }

    #[test]
    fn a_boulevard_pushes_the_carriageways_apart() {
        let layout = LaneLayout::divided(1, 1, MEDIAN_BOULEVARD_WIDTH_M);
        // Lanes are: backward, median, forward.
        let back = layout.centre_offset(0).expect("lane exists");
        let fwd = layout.centre_offset(2).expect("lane exists");
        let gap = fwd - back;
        assert!(
            (gap - (config::LANE_WIDTH + MEDIAN_BOULEVARD_WIDTH_M)).abs() < 1e-6,
            "carriageway centres {gap} m apart"
        );
    }

    #[test]
    fn an_ordinary_road_never_leaves_the_stack() {
        // The road tool caps each direction at four, so these are the layouts
        // that actually occur. None of them should reach the heap.
        for (fwd, bkw) in [(1u8, 1u8), (2, 2), (0, 1), (4, 0)] {
            let layout = LaneLayout::from_counts(fwd, bkw);
            assert!(
                !layout.spilled(),
                "{fwd}/{bkw} spilled to the heap"
            );
        }
        // A dual carriageway with a median still fits.
        assert!(!LaneLayout::divided(1, 1, 2.0).spilled());
        // The widest authorable road does spill, which is fine and rare.
        assert!(LaneLayout::divided(4, 4, 3.0).spilled());
    }

    #[test]
    fn walking_the_layout_matches_the_old_formula_on_symmetric_roads() {
        // The geometry builder walks forward lanes in order and backward lanes
        // in reverse, so index -1 lands nearest the centre. On a symmetric
        // road that reproduces the formula it replaced exactly:
        //   forward lane l at  (l + 0.5) * LANE_WIDTH
        //   backward lane l at -(l + 0.5) * LANE_WIDTH
        //
        // Only symmetric counts are checked here, because that is the only
        // case where the old formula was self-consistent. The asymmetric case
        // is the next test, and it is a deliberate behaviour change.
        let w = config::LANE_WIDTH;
        for n in 1u8..=4 {
            let layout = LaneLayout::from_counts(n, n);

            let mut seen = 0i8;
            for (band, lane) in layout.lanes().iter().enumerate() {
                if lane.direction != LaneDirection::Forward {
                    continue;
                }
                let got = layout.centre_offset(band).expect("band exists");
                let want = (f32::from(seen as u8) + 0.5) * w;
                assert!((got - want).abs() < 1e-6, "fwd {seen} of {n}/{n}: {got} wanted {want}");
                seen += 1;
            }

            let mut seen = 0i8;
            for (band, lane) in layout.lanes().iter().enumerate().rev() {
                if lane.direction != LaneDirection::Backward {
                    continue;
                }
                let got = layout.centre_offset(band).expect("band exists");
                let want = -(f32::from(seen as u8) + 0.5) * w;
                assert!((got - want).abs() < 1e-6, "bkw {seen} of {n}/{n}: {got} wanted {want}");
                seen += 1;
            }
        }
    }

    #[test]
    fn an_asymmetric_road_is_centred_on_itself_rather_than_on_the_direction_split() {
        // A deliberate behaviour change, and the reason for it is that the old
        // formula disagreed with the sidewalks beside it.
        //
        // The old form pinned lane offsets to the centreline and let the
        // carriageway grow lopsided, so a four-lane one-way put every lane on
        // one side. But the sidewalks were always placed at plus and minus half
        // the asphalt width, meaning they straddled the middle of the road
        // while the lanes did not. On a symmetric road nobody noticed. On an
        // asymmetric one the lanes and the kerb disagreed.
        //
        // Accumulating real widths centres the carriageway on its own
        // alignment, which is what the sidewalks already assumed.
        for (fwd, bkw) in [(3u8, 1u8), (4, 0), (1, 3), (0, 2)] {
            let layout = LaneLayout::from_counts(fwd, bkw);
            let half = layout.asphalt_width() * 0.5;

            let mut lo = f32::MAX;
            let mut hi = f32::MIN;
            for band in 0..layout.lanes().len() {
                let o = layout.centre_offset(band).expect("band exists");
                lo = lo.min(o);
                hi = hi.max(o);
            }
            if layout.lanes().is_empty() {
                continue;
            }
            // The outermost lane centres sit half a lane inside each kerb, so
            // the carriageway is symmetric about zero whatever the split.
            let left_gap = lo + half;
            let right_gap = half - hi;
            assert!(
                (left_gap - right_gap).abs() < 1e-6,
                "{fwd}/{bkw} is off centre: {left_gap} against {right_gap}"
            );
        }
    }

    #[test]
    fn a_turn_angle_classifies_into_a_movement() {
        use std::f32::consts::PI;
        assert_eq!(TurnSet::movement_for_angle(0.0), TurnSet::THROUGH);
        // A gently curving road is still a through movement.
        assert_eq!(TurnSet::movement_for_angle(0.3), TurnSet::THROUGH);
        assert_eq!(TurnSet::movement_for_angle(-0.3), TurnSet::THROUGH);
        assert_eq!(TurnSet::movement_for_angle(PI / 2.0), TurnSet::LEFT);
        assert_eq!(TurnSet::movement_for_angle(-PI / 2.0), TurnSet::RIGHT);
        assert_eq!(TurnSet::movement_for_angle(PI), TurnSet::U_TURN);
        assert_eq!(TurnSet::movement_for_angle(-PI), TurnSet::U_TURN);
    }

    #[test]
    fn a_left_only_pocket_refuses_the_through_movement() {
        use std::f32::consts::PI;
        let pocket = TurnSet(TurnSet::LEFT);
        assert!(pocket.allows_angle(PI / 2.0));
        assert!(!pocket.allows_angle(0.0));
        assert!(!pocket.allows_angle(-PI / 2.0));

        // And a through lane refuses the left, which is the other half of what
        // makes a turn pocket mean anything.
        let straight = TurnSet(TurnSet::THROUGH);
        assert!(straight.allows_angle(0.0));
        assert!(!straight.allows_angle(PI / 2.0));
    }

    #[test]
    fn an_unrestricted_lane_still_allows_every_angle() {
        use std::f32::consts::PI;
        for a in [-PI, -PI / 2.0, 0.0, PI / 2.0, PI] {
            assert!(TurnSet::ANY.allows_angle(a), "angle {a} refused");
        }
    }

    #[test]
    fn an_unrestricted_turn_set_allows_everything() {
        assert!(TurnSet::ANY.allows(TurnSet::LEFT));
        assert!(TurnSet::ANY.allows(TurnSet::THROUGH));
        let left_only = TurnSet(TurnSet::LEFT);
        assert!(left_only.allows(TurnSet::LEFT));
        assert!(!left_only.allows(TurnSet::THROUGH));
    }

    #[test]
    fn a_bus_lane_refuses_a_car_and_an_ordinary_lane_takes_a_bus() {
        // The whole feature is the mode bit withheld. Geometry is identical,
        // which is why a bus lane must not be a different kind of band.
        let bus = LaneSpec::bus(LaneDirection::Forward);
        assert!(bus.carries(TransitFlags::BUS));
        assert!(!bus.carries(TransitFlags::CAR));
        assert_eq!(bus.width_m, config::LANE_WIDTH);

        let ordinary = LaneSpec::travel(LaneDirection::Forward);
        assert!(ordinary.carries(TransitFlags::CAR));
        assert!(
            ordinary.carries(TransitFlags::BUS),
            "a bus uses an ordinary lane when there is no bus lane"
        );
    }

    #[test]
    fn a_part_time_bus_lane_is_a_lane_a_car_can_be_on() {
        // It has to be built as a lane cars may occupy, because outside the
        // hours it is in force they do.
        let lane = LaneSpec::bus_part_time(LaneDirection::Forward);
        assert!(lane.carries(TransitFlags::CAR));
        assert!(lane.carries(TransitFlags::BUS));
    }

    #[test]
    fn a_cycle_track_is_narrower_than_a_traffic_lane_and_carries_only_bikes() {
        let track = LaneSpec::cycle_track(LaneDirection::Forward);
        assert!(track.carries(TransitFlags::BIKE));
        assert!(!track.carries(TransitFlags::CAR));
        assert!(
            track.width_m < config::LANE_WIDTH,
            "a cycle track that takes a full lane is what stops cities building them"
        );
    }

    #[test]
    fn parking_takes_width_and_carries_nothing() {
        let bay = LaneSpec::parking();
        assert!(!bay.is_travel());
        assert!(!bay.carries(TransitFlags::CAR));
        assert!(bay.width_m > 0.0);

        // And it widens the road it sits on by exactly its own width.
        let plain = LaneLayout::from_counts(1, 1);
        let parked = LaneLayout::with_parking(1, 1);
        let delta = parked.asphalt_width() - plain.asphalt_width();
        assert!((delta - PARKING_WIDTH_M * 2.0).abs() < 1e-6);
        assert!((parked.parking_width() - PARKING_WIDTH_M * 2.0).abs() < 1e-6);
    }

    #[test]
    fn a_turn_pocket_is_absent_at_the_start_and_live_at_the_node() {
        let pocket = LaneSpec::turn_pocket(LaneDirection::Forward, TurnSet(TurnSet::LEFT), 0.25);
        assert!(!pocket.range.contains(0.0), "a pocket does not run the whole edge");
        assert!(pocket.range.contains(1.0), "a pocket is live where the queue forms");
        assert!(pocket.turns.allows(TurnSet::LEFT));
        assert!(!pocket.turns.allows(TurnSet::THROUGH));

        let layout = LaneLayout::from_lanes([
            LaneSpec::travel(LaneDirection::Forward),
            pocket,
        ]);
        assert!(!layout.all_full_length());
        assert_eq!(layout.live_at(0.0).count(), 1, "only the through lane exists early");
        assert_eq!(layout.live_at(1.0).count(), 2, "the pocket has opened by the node");
    }

    #[test]
    fn a_pedestrianised_street_keeps_its_lanes_and_loses_its_cars() {
        // The point of the whole design: it is a street with a restriction,
        // not a footpath, so deliveries still arrive.
        let street = LaneLayout::pedestrianised(1, 1, TransitFlags::FOOT | TransitFlags::BIKE);
        assert!(street.is_car_free());
        assert!(!street.is_empty(), "it is still a road");
        assert!(
            (street.asphalt_width() - LaneLayout::from_counts(1, 1).asphalt_width()).abs() < 1e-6,
            "restricting who may drive does not change how wide the road is"
        );
    }

    #[test]
    fn counting_lanes_depends_on_who_is_asking() {
        // A road with a bus lane has more lanes for a bus than for a car, and
        // that difference is the reason count_for_mode exists.
        let layout = LaneLayout::from_lanes([
            LaneSpec::travel(LaneDirection::Forward),
            LaneSpec::bus(LaneDirection::Forward),
        ]);
        assert_eq!(layout.count_for_mode(LaneDirection::Forward, TransitFlags::CAR), 1);
        assert_eq!(layout.count_for_mode(LaneDirection::Forward, TransitFlags::BUS), 2);
        assert_eq!(layout.fwd_count(), 1, "fwd_count is the car answer");
    }

    #[test]
    fn a_painted_median_lies_flat_and_a_built_one_does_not() {
        // What separates a line on the road from a kerbed island, and the only
        // thing that decides whether a vehicle may turn across it.
        let painted = LaneSpec::median_painted();
        assert!(!painted.blocks_turns_across());
        let built = LaneSpec::median(2.0);
        assert!(built.blocks_turns_across());
        assert!(!built.is_boulevard());
        assert!(LaneSpec::median(MEDIAN_BOULEVARD_WIDTH_M).is_boulevard());
    }

    #[test]
    fn a_three_lane_road_has_a_centre_that_belongs_to_neither_direction() {
        // The reason an odd lane count exists. A two-way left-turn lane is
        // entered from both sides, so it is in neither total, and the counts
        // deliberately do not sum to the lane count.
        let layout = LaneLayout::with_two_way_left_turn(1, 1);
        assert_eq!(layout.lanes().len(), 3);
        assert_eq!(layout.fwd_count(), 1);
        assert_eq!(layout.bkw_count(), 1);
        assert!(layout.has_reversible());

        let centre = layout.lanes()[1];
        assert!(centre.is_reversible());
        assert_eq!(centre.direction, LaneDirection::None);
        assert!(
            centre.carries(TransitFlags::CAR),
            "a car must be able to occupy it, or it is a median"
        );
        assert!(centre.turns.allows(TurnSet::LEFT));
        assert!(!centre.turns.allows(TurnSet::THROUGH));
    }

    #[test]
    fn an_asymmetric_road_is_legal_and_centred_on_itself() {
        // Two lanes one way and one the other is an ordinary arrangement, not
        // a rounding error, and the carriageway is symmetric about its own
        // alignment whatever the split.
        let layout = LaneLayout::from_counts(2, 1);
        assert_eq!(layout.fwd_count(), 2);
        assert_eq!(layout.bkw_count(), 1);
        let half = layout.asphalt_width() * 0.5;
        let lo = layout.centre_offset(0).expect("first band");
        let hi = layout
            .centre_offset(layout.lanes().len() - 1)
            .expect("last band");
        assert!(((lo + half) - (half - hi)).abs() < 1e-6, "off centre");
    }

    #[test]
    fn a_tidal_lane_flips_without_moving() {
        // Reversing the peak direction changes who may use the band and
        // nothing about where it is, which is what makes it cheap to do twice
        // a day.
        let mut layout = LaneLayout::tidal(1, 1, LaneDirection::Forward);
        let width_before = layout.asphalt_width();
        let offsets_before: Vec<f32> = (0..layout.lanes().len())
            .filter_map(|i| layout.centre_offset(i))
            .collect();
        assert_eq!(layout.fwd_count(), 2, "the tide runs with the forward lanes");
        assert_eq!(layout.bkw_count(), 1);

        layout.set_tidal_direction(LaneDirection::Backward);
        assert_eq!(layout.fwd_count(), 1);
        assert_eq!(layout.bkw_count(), 2, "the tide has turned");

        assert!((layout.asphalt_width() - width_before).abs() < 1e-6);
        let offsets_after: Vec<f32> = (0..layout.lanes().len())
            .filter_map(|i| layout.centre_offset(i))
            .collect();
        assert_eq!(offsets_before, offsets_after, "the band did not move");
    }

    #[test]
    fn turn_pockets_widen_the_approach_and_nothing_else() {
        // A road that carries two lanes between junctions arrives at one with
        // four. The count differs at the two ends, which is the other thing a
        // pair of lane counts cannot describe.
        let plain = LaneLayout::from_counts(2, 2);
        let approach = LaneLayout::from_counts(2, 2)
            .with_turn_pockets(1, 1, LaneDirection::Forward, 0.25);

        assert_eq!(approach.lanes().len(), 6);
        assert!(!approach.all_full_length());

        // Early on the road is exactly what it was.
        assert!(
            (approach.asphalt_width_at(0.0) - plain.asphalt_width()).abs() < 1e-6,
            "the pockets have not opened yet"
        );
        // At the node it is two lanes wider.
        let widened = approach.asphalt_width_at(1.0) - plain.asphalt_width();
        assert!(
            (widened - config::LANE_WIDTH * 2.0).abs() < 1e-6,
            "expected two extra lanes at the stop line, got {widened}"
        );
        // And the roadbed still has to reserve the widest case.
        assert!(
            (approach.asphalt_width() - approach.asphalt_width_at(1.0)).abs() < 1e-6
        );

        assert_eq!(approach.live_at(0.0).count(), 4);
        assert_eq!(approach.live_at(1.0).count(), 6);
    }

    #[test]
    fn angling_parking_trades_roadway_depth_for_curb_frontage() {
        // The whole reason a street is marked at an angle. Ninety degree bays
        // fit more than twice the cars of parallel along the same curb, and
        // take more than twice the depth to do it.
        let curb = 60.0_f32;
        let parallel = ParkingAngle::Parallel;
        let ninety = ParkingAngle::Perpendicular90;

        assert!(ninety.spaces_along(curb) > parallel.spaces_along(curb) * 2);
        assert!(ninety.depth_m() > parallel.depth_m() * 2.0);

        // Forty-five sits between the two on both counts, which is why it is
        // the common compromise.
        let angled = ParkingAngle::Angled45;
        assert!(angled.spaces_along(curb) > parallel.spaces_along(curb));
        assert!(angled.spaces_along(curb) < ninety.spaces_along(curb));
        assert!(angled.depth_m() > parallel.depth_m());
        assert!(angled.depth_m() < ninety.depth_m());
    }

    #[test]
    fn a_parking_band_is_as_deep_as_its_angle_says() {
        for angle in [
            ParkingAngle::Parallel,
            ParkingAngle::Angled45,
            ParkingAngle::Perpendicular90,
        ] {
            let lane = LaneSpec::parking_at(angle);
            assert_eq!(lane.kind, LaneKind::Parking);
            assert!((lane.width_m - angle.depth_m()).abs() < 1e-6);
            assert!(!lane.is_moving(), "parking carries no moving traffic");
            assert_eq!(lane.parking_angle, angle);
        }
    }

    #[test]
    fn only_parking_bands_yield_spaces() {
        // A travel lane is not parking however wide it is, and a count that
        // said otherwise would double every supply figure.
        assert_eq!(
            LaneSpec::travel(LaneDirection::Forward).parking_spaces_along(100.0),
            0
        );
        assert_eq!(LaneSpec::verge(2.0).parking_spaces_along(100.0), 0);
        assert!(LaneSpec::parking().parking_spaces_along(100.0) > 0);
    }

    #[test]
    fn a_verge_takes_width_and_carries_nothing() {
        let verge = LaneSpec::verge(VERGE_WIDTH_M);
        assert_eq!(verge.kind, LaneKind::Verge);
        assert!(!verge.is_moving());
        assert!(!verge.carries(TransitFlags::FOOT));
        assert!(!verge.carries(TransitFlags::CAR));

        // And it widens the street by exactly what it occupies, without
        // touching the carriageway.
        let plain = LaneLayout::from_counts(1, 1);
        let planted = LaneLayout::with_parking_and_verge(
            1,
            1,
            ParkingAngle::Parallel,
            VERGE_WIDTH_M,
        );
        let added = planted.asphalt_width() - plain.asphalt_width();
        let expected = (PARKING_WIDTH_M + VERGE_WIDTH_M) * 2.0;
        assert!((added - expected).abs() < 1e-6, "added {added}, wanted {expected}");
        assert!((planted.verge_width() - VERGE_WIDTH_M * 2.0).abs() < 1e-6);
        assert_eq!(planted.fwd_count(), 1, "the carriageway did not change");
        assert_eq!(planted.bkw_count(), 1);
    }

    #[test]
    fn a_street_counts_both_curbs_when_supplying_parking() {
        let curb = 60.0_f32;
        let layout =
            LaneLayout::with_parking_and_verge(1, 1, ParkingAngle::Angled45, VERGE_WIDTH_M);
        let one_side = ParkingAngle::Angled45.spaces_along(curb);
        assert_eq!(layout.parking_spaces_along(curb), one_side * 2);
    }

    #[test]
    fn sidewalk_width_is_authored_or_default() {
        let plain = LaneLayout::from_counts(1, 1);
        assert!(plain.authored_sidewalk_width().is_none());
        assert!((plain.sidewalk_width() - config::SIDEWALK_WIDTH).abs() < 1e-6);

        // A high street authors a wide one, and the record keeps the fact that
        // it was authored so a later change to the default does not move it.
        let promenade = LaneLayout::from_counts(1, 1).with_sidewalk_width(6.0);
        assert_eq!(promenade.authored_sidewalk_width(), Some(6.0));
        assert!((promenade.sidewalk_width() - 6.0).abs() < 1e-6);
    }

    #[test]
    fn a_cross_section_survives_the_godot_boundary() {
        // The editor reads a layout out, the player edits it, and it comes
        // back. If the round trip loses a band or a property, the editor
        // silently rewrites roads it was only meant to display.
        let original = LaneLayout::from_lanes([
            LaneSpec::verge(VERGE_WIDTH_M),
            LaneSpec::parking_at(ParkingAngle::Angled45),
            LaneSpec::travel(LaneDirection::Backward),
            LaneSpec::two_way_left_turn(),
            LaneSpec::bus(LaneDirection::Forward),
            LaneSpec::cycle_track(LaneDirection::Forward),
        ]);

        let flat = original.to_flat();
        assert_eq!(flat.len(), original.lanes().len() * 7);

        let restored = LaneLayout::from_flat(&flat).expect("well-formed");
        assert_eq!(restored.lanes().len(), original.lanes().len());
        for (a, b) in restored.lanes().iter().zip(original.lanes().iter()) {
            assert_eq!(a.kind, b.kind);
            assert_eq!(a.direction, b.direction);
            assert_eq!(a.modes, b.modes);
            assert_eq!(a.marking, b.marking);
            assert_eq!(a.turns, b.turns);
            assert_eq!(a.parking_angle, b.parking_angle);
            assert!((a.width_m - b.width_m).abs() < 1e-3, "width drifted");
        }
        assert!((restored.asphalt_width() - original.asphalt_width()).abs() < 1e-3);
    }

    #[test]
    fn a_malformed_cross_section_is_refused_rather_than_half_built() {
        // Half a cross-section is worse than none, so every one of these
        // returns nothing rather than a partial road.
        assert!(LaneLayout::from_flat(&[]).is_none(), "empty");
        assert!(LaneLayout::from_flat(&[0, 0, 3500, 2, 0, 0]).is_none(), "short");
        assert!(
            LaneLayout::from_flat(&[99, 0, 3500, 2, 0, 0, 0]).is_none(),
            "unknown kind"
        );
        assert!(
            LaneLayout::from_flat(&[0, 9, 3500, 2, 0, 0, 0]).is_none(),
            "unknown direction"
        );
        assert!(
            LaneLayout::from_flat(&[0, 0, 3500, 2, 9, 0, 0]).is_none(),
            "unknown marking"
        );
        assert!(
            LaneLayout::from_flat(&[0, 0, 3500, 2, 0, 0, 9]).is_none(),
            "unknown parking angle"
        );
        assert!(
            LaneLayout::from_flat(&[0, 0, 0, 2, 0, 0, 0]).is_none(),
            "zero width"
        );
    }
}
