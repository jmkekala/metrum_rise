// SPDX-License-Identifier: GPL-2.0-only

//! Junction layout (`ROAD-68-C`, part 1): the arm order, sectors, curb-return corners, round
//! joins, mouth stations and carriageway core loop of one node, built from abstract arms.
//!
//! The layout is pure. It reads only the arms passed to [`JunctionLayout::build`], keeps no state
//! between calls and does not know the graph or the span pieces. `docs/roads.md` ("Junction
//! layout") owns the contract; this header records the conventions and the choices it leaves to
//! code.
//!
//! # Conventions
//!
//! Geometry is world XZ in `f64`, stored as [`RoadVec2`] `(x, z)`. Angles are `atan2(z, x)`.
//! "Counter-clockwise" means increasing angle, and the left of a direction `d` is `d.perp()`, a
//! quarter turn counter-clockwise. Each arm runs outward from the node. Its lateral axes point to
//! its left, and signed lateral offsets are positive on the left and negative on the right. Band
//! offsets in [`ArmBand`] are unsigned distances from the centreline. In the legacy section frame
//! (`lateral_xz = tangent.perp()`, [`RoadBand`](super::RoadBand) offsets negative on the left),
//! this left is the positive side of the lateral axis, which `RoadBand` calls right, for an arm
//! leaving from its edge's start; for an arm at its edge's end, both the axis and the sides swap.
//!
//! The offset curve `o(s, l)` of an arm is piecewise linear in the station `s`. Between knots `k`
//! and `k + 1` it interpolates `knot_k + lateral_k * l` and `knot_k+1 + lateral_k+1 * l`
//! ([`ArmInput::offset_point`]). Every point the layout takes from an arm comes from that one
//! function, so endpoints are copied, never re-derived.
//!
//! # Construction
//!
//! 1. Arms sort counter-clockwise by their direction over the first metre, then by [`ArmKey`].
//! 2. Sector `i` runs from arm `i`'s left side to arm `i + 1`'s right side. With angle `phi`, it
//!    is concave if `phi < pi - eps`, straight if `|phi - pi| <= eps` and convex beyond. A
//!    terminal has one convex sector of `2 pi`.
//! 3. A concave sector gets a curb-return corner. Its centre `C` is the first crossing, by
//!    `max(sa, sb)` and then search order, of `o_a(., la + R)` and `o_b(., -(lb + R))`, over
//!    stations up to each arm's `max_station_m`. Boundary `j` runs from `o_a(sa, la_j)` to
//!    `o_b(sb, -lb_j)`; its angle and radius about `C` interpolate linearly between those exact
//!    endpoints.
//! 4. Straight and convex sectors get a round join about the node: boundary `j` runs from
//!    `o_a(sa_j, la_j)` to `o_b(sb_j, -lb_j)`, with the radius interpolated linearly by angle. The
//!    stations are zero except in a straight sector, where each boundary starts where its offset
//!    curve crosses the sector bisector: ahead of the node when the sector is narrower than `pi`
//!    and the arms overlap, on the first offset segment extended behind the node otherwise. The
//!    arms meet in a miter on the bisector; a boundary with equal offsets on both arms is the one
//!    miter point, taken from the `from_arm` curve. Joining at station zero would make the core
//!    loop cross itself below `pi` whenever the widths are equal, and leave a rounding-sized arc
//!    of sliver strips above it (`ROAD-68` open questions 5 and 9).
//! 5. A straight sector between unequal boundary offsets tapers (D4): each such boundary meets
//!    the sector at the mean of its two offsets, so the join is that of equal widths, and runs
//!    linearly in station to its arm's own offset at the taper station on both approaches
//!    ([`BoundaryTaper`]). The taper takes half the legacy span taper length on each arm,
//!    shortened to fit both arms beside their crosswalks; shorter than the legacy minimum, the
//!    layout fails with [`LayoutError::ArmTooShort`].
//! 6. An arm's mouth station is the largest of `MIN_MOUTH_STATION_M` and the stations at which
//!    its two sides reach their own offsets, plus `CROSSWALK_DEPTH_M` on crossable arms (D8). That
//!    depth is the arm's [`Crosswalk`] strip, on the straight approach right outside the corners;
//!    its crossing line is the middle of the depth, and its kerb points are the two
//!    carriageway-edge points there. An attached arm whose edge meets an arm side moves only that
//!    arm's mouth, to `MIN_MOUTH_STATION_M` beyond the meeting point when that is farther; the
//!    zebra stays at the corner (`ROAD-68` open question 17).
//! 7. A corner that would push a mouth past `max_station_m` shrinks its radius by bisection
//!    towards the corner's raised width plus `CURB_RETURN_CLEARANCE_M` (D1). If that minimum does
//!    not fit either, layout fails with [`LayoutError::CornerDoesNotFit`].
//! 8. The carriageway core loop runs counter-clockwise: out along each arm's right carriageway
//!    edge, across its mouth, back along its left edge and through the next sector's core-edge
//!    samples. A loop that touches or crosses itself fails with [`LayoutError::CoreNotSimple`].
//!
//! Bands beyond the carriageway edge match across a sector by their position outward. A band
//! present on one side only has zero width on the other and tapers along the sector. Matched
//! bands must agree in kind and level ([`LayoutError::BandMismatch`]).
//!
//! # Complexity
//!
//! `O(k log k + sum over corners of (Pa + Pb) + V)`, for degree `k`, `Pa` and `Pb` the knots of a
//! corner's two arms inside the setback, and `V` emitted samples. The corner search stops at the
//! first crossing by station, so a setback's few knots give constant work per tested segment
//! pair. Two arms whose offset curves do not cross near the node make the search test every
//! segment pair up to `max_station_m`, `O(Pa * Pb)`, repeated a fixed number of times by the
//! radius bisection. The core simplicity check tests segment pairs only between loop pieces
//! whose bounds overlap: linear on product junctions, quadratic in the worst case.
//!
//! [`JunctionLayout::build`] allocates its output vectors and working buffers. Rebuilding a layout
//! in place (`JunctionLayout::rebuild`) with a reused [`LayoutScratch`] allocates nothing once
//! both have grown to the node's degree and samples (`ROAD-68-K18`).

use super::span::{
    WIDTH_CHANGE_TAPER_MAX_M, WIDTH_CHANGE_TAPER_MIN_M, WIDTH_CHANGE_TAPER_WIDTH_FACTOR,
};
use super::{BandKind, Level, RoadVec2};
use std::f64::consts::{PI, TAU};
use std::ops::Range;

// D1: the curb-return radius at the carriageway edge before fitting (user-confirmed 2026-10-24).
const CURB_RETURN_RADIUS_M: f64 = 3.5;
// D1: the smallest radius is the corner's raised width plus this clearance, which keeps the
// outermost boundary at least this far from the corner centre.
const CURB_RETURN_CLEARANCE_M: f64 = 0.5;
// D8: on crossable arms the mouth moves this far beyond its sectors; the zebra strip fills it.
// A pass-through node's crossing strip on its span has the same depth.
pub(super) const CROSSWALK_DEPTH_M: f64 = 3.0;
// s_min: every node piece reaches at least this far along each arm, so a mouth never sits on the
// node's lateral line where round joins start.
const MIN_MOUTH_STATION_M: f64 = 1.0;
// eps: sectors within one degree of a half turn are straight. That covers the direction noise of
// a straight road split at a junction anywhere in a 20 km world (f32 knots, below about 1e-3 rad
// over the first metre) and treats visually straight continuations as straight, so a width change
// there tapers (D4) rather than needing a corner that cannot exist.
const STRAIGHT_SECTOR_EPS_RAD: f64 = PI / 180.0;
// Consecutive arms closer than this are duplicates. ROAD-54 rejects junction arms below 25 degrees
// and bends below 10 degrees long before this.
const MIN_SECTOR_ANGLE_RAD: f64 = PI / 180.0;
// Arm direction over the first metre, as `validate.rs` measures it, and the shortest run that
// still defines a direction (its sub-5 cm segment rule).
const ARM_DIRECTION_RUN_M: f64 = 1.0;
const MIN_DIRECTION_RUN_M: f64 = 0.05;
// Curve sampling: no sampled boundary segment is longer than this or turns more than this.
const MAX_SAMPLE_SEGMENT_M: f64 = 0.5;
const MAX_SAMPLE_ANGLE_RAD: f64 = PI / 18.0;
// Radius bisection steps; 20 halvings resolve any radius range below 4 m to under 4 micrometres.
const RADIUS_BISECTION_STEPS: usize = 20;
// Lateral axes must be unit length to this tolerance on their squared length.
const LATERAL_UNIT_TOLERANCE: f64 = 1e-9;

/// The end of its edge at which an arm leaves the node.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum EdgeEnd {
    /// The edge's start node.
    Start,
    /// The edge's end node.
    End,
}

/// Stable arm identity; arms with equal directions sort by it.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ArmKey {
    /// Graph edge index.
    pub edge: usize,
    /// Edge end at this node.
    pub end: EdgeEnd,
}

/// One band on one side of an arm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArmBand {
    /// Band classification.
    pub kind: BandKind,
    /// Band height level.
    pub level: Level,
    /// Unsigned distance from the centreline to the band's outer boundary, in metres.
    pub outer_m: f64,
}

/// The bands on one side of an arm, listed outward from the centreline.
///
/// The first `core_bands` bands form the carriageway core, so the outer boundary of band
/// `core_bands - 1` is the carriageway edge. A footpath-only arm has one band on each side, which
/// is its core.
#[derive(Clone, Copy, Debug)]
pub struct ArmSide<'a> {
    /// Bands outward from the centreline, with strictly increasing outer offsets.
    pub bands: &'a [ArmBand],
    /// Number of leading bands inside the carriageway edge; at least one.
    pub core_bands: usize,
}

/// One centreline knot of an arm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArmKnot {
    /// Centreline position in world XZ.
    pub position: RoadVec2,
    /// Unit lateral axis, pointing to the left of the outward direction.
    pub lateral: RoadVec2,
    /// Centreline distance from the node, in metres.
    pub station_m: f64,
}

/// One arm of a node: an incident edge end with its centreline and cross-section.
#[derive(Clone, Copy, Debug)]
pub struct ArmInput<'a> {
    /// Stable identity, used for sorting ties and error reports.
    pub key: ArmKey,
    /// Centreline oriented away from the node. The first knot is the node at station zero, and
    /// stations increase strictly.
    pub knots: &'a [ArmKnot],
    /// Bands on the left of the outward direction.
    pub left: ArmSide<'a>,
    /// Bands on the right of the outward direction.
    pub right: ArmSide<'a>,
    /// Furthest station this end may use, half the edge length; at most the last knot station.
    pub max_station_m: f64,
    /// Whether the mouth carries a crosswalk (D8).
    pub crossable: bool,
    /// Whether the arm bounds no sector and instead joins the outer boundary of the sector it
    /// lies in: a footpath at a road node (D5).
    pub attached: bool,
}

/// Degree class of a node.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeKind {
    /// One arm, closed by a round cap (D2).
    Terminal,
    /// Two arms.
    Bend,
    /// Three or more arms.
    Junction,
}

/// Classification of the sector between two consecutive arms.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SectorKind {
    /// Narrower than a half turn: a curb-return corner.
    Concave,
    /// Within the straight tolerance of a half turn: a round join or miter at the node.
    Straight,
    /// Wider than a half turn, including a terminal's full turn: a round join about the node.
    Convex,
}

/// Side of an arm, seen walking away from the node.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
    /// Positive lateral offsets.
    Left,
    /// Negative lateral offsets.
    Right,
}

/// The concave corner on one side of an arm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CornerTangent {
    /// Station at which every boundary on this side leaves its offset curve for the corner.
    pub station_m: f64,
    /// Curb-return radius the corner uses after fitting.
    pub radius_m: f64,
}

/// Layout products of one arm.
#[derive(Clone, Debug, PartialEq)]
pub struct ArmLayout {
    /// Arm identity.
    pub key: ArmKey,
    /// Index of the arm in the slice passed to [`JunctionLayout::build`].
    pub input_index: usize,
    /// Unit direction over the first metre of the centreline.
    pub direction: RoadVec2,
    /// Direction angle in `[0, 2 pi)`.
    pub angle_rad: f64,
    /// Station of the mouth section, where the node piece ends and the span begins.
    pub mouth_station_m: f64,
    /// Zebra strip of a crossable arm (D8); its outer edge is the mouth unless an attached arm
    /// moved the mouth farther out.
    pub crosswalk: Option<Crosswalk>,
    /// Concave corner on the left side, toward the next arm.
    pub left_corner: Option<CornerTangent>,
    /// Concave corner on the right side, toward the previous arm.
    pub right_corner: Option<CornerTangent>,
}

/// The zebra strip of a crossable arm (D8): `CROSSWALK_DEPTH_M` deep across the arm's straight
/// approach, from where both sides have left their sectors (the curbs stop curving) outward.
/// Pedestrian lanes, zebra markings and node pieces all read it from here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crosswalk {
    /// Station of the strip's inner edge: the larger of `s_min` and the stations at which the
    /// arm's two sides leave their sectors.
    pub inner_station_m: f64,
    /// Station of the crossing line, the middle of the strip.
    pub station_m: f64,
    /// Carriageway-edge points on the crossing line, right side first:
    /// `o(station, -right edge)` and `o(station, left edge)`. The crossing is square to the
    /// arm's section frame there.
    pub kerbs: [RoadVec2; 2],
}

/// An attached arm (D5): its ribbon ends on the outer boundary of the sector it lies in, on
/// shared vertices, and its mouth lies beyond that.
#[derive(Clone, Debug, PartialEq)]
pub struct Attachment {
    /// Arm identity.
    pub key: ArmKey,
    /// Index of the arm in the slice passed to [`JunctionLayout::build`].
    pub input_index: usize,
    /// Unit direction over the first metre of the centreline.
    pub direction: RoadVec2,
    /// Index of the sector it lies in.
    pub sector: usize,
    /// Station of its mouth section: `MIN_MOUTH_STATION_M` beyond its farther edge point.
    pub mouth_station_m: f64,
    /// Where its right and left edges meet the sector's outer boundary.
    pub edges: [AttachedEdge; 2],
}

/// Where one edge of an attached arm meets its sector's outer boundary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AttachedEdge {
    /// Signed lateral offset of the edge on the attached arm.
    pub offset_m: f64,
    /// Station on the attached arm of the meeting point.
    pub station_m: f64,
    /// The meeting point: a vertex of the outer boundary, copied by both pieces of geometry.
    pub point: RoadVec2,
    /// Where on the boundary the point lies.
    pub on: BoundaryPoint,
}

/// A point of a sector's outermost boundary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoundaryPoint {
    /// A sample, by index into [`JunctionLayout::samples`]; inserted into every boundary of the
    /// sector at the same position, so the sector's strips stay quads.
    Sample(usize),
    /// A station on an arm side's run from the sector to the mouth, inserted into the runs of
    /// every boundary beyond the carriageway edge.
    Run {
        /// Sorted arm index.
        arm: usize,
        /// Arm side.
        side: Side,
        /// Station on the arm.
        station_m: f64,
    },
}

/// One point of a boundary run or outer path, with its station on the arm it follows and, at a
/// mouth corner, the arm (input order) and lateral offset of that corner.
#[derive(Clone, Copy, Debug)]
pub(super) struct PathPoint {
    pub(super) station_m: f64,
    pub(super) xz: RoadVec2,
    pub(super) mouth: Option<(usize, f64)>,
}

impl PathPoint {
    fn at(station_m: f64, xz: RoadVec2) -> Self {
        Self {
            station_m,
            xz,
            mouth: None,
        }
    }
}

/// A fitted curb-return corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Corner {
    /// Radius of the carriageway-edge curve.
    pub radius_m: f64,
    /// Centre of the polar interpolation.
    pub centre: RoadVec2,
}

/// Layout products of one sector.
#[derive(Clone, Debug, PartialEq)]
pub struct SectorLayout {
    /// Sector classification.
    pub kind: SectorKind,
    /// Sorted index of the arm whose left side bounds the sector.
    pub from_arm: usize,
    /// Sorted index of the arm whose right side bounds the sector.
    pub to_arm: usize,
    /// Angle between the two arm directions, in `(0, 2 pi]`.
    pub angle_rad: f64,
    /// Fitted corner of a concave sector.
    pub corner: Option<Corner>,
    /// Range into [`JunctionLayout::boundaries`], from the carriageway edge outward; every
    /// boundary of the sector has one sample count, both endpoints included.
    pub boundaries: Range<usize>,
}

/// Band on one side of a sector boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BoundaryBand {
    /// Band classification.
    pub kind: BandKind,
    /// Band height level.
    pub level: Level,
}

/// One sampled band boundary across a sector.
#[derive(Clone, Debug, PartialEq)]
pub struct SectorBoundary {
    /// Band inside the boundary; the core band for the carriageway edge.
    pub inner: BoundaryBand,
    /// Band outside the boundary, or `None` on the outermost boundary.
    pub outer: Option<BoundaryBand>,
    /// Station at which the boundary leaves the `from_arm` offset curve.
    pub from_station_m: f64,
    /// Signed lateral offset of the boundary on the `from_arm` (left, positive).
    pub from_offset_m: f64,
    /// Station at which the boundary joins the `to_arm` offset curve.
    pub to_station_m: f64,
    /// Signed lateral offset of the boundary on the `to_arm` (right, negative).
    pub to_offset_m: f64,
    /// Width-change taper of a straight sector (D4): the offsets above are then the mean of the
    /// two arms' own offsets, and the boundary reaches each arm's own offset at the taper station.
    pub taper: Option<BoundaryTaper>,
    /// Range into [`JunctionLayout::samples`], from the `from_arm` endpoint to the `to_arm` one.
    pub samples: Range<usize>,
}

/// Where a tapered sector boundary (D4) reaches the two arms' own offsets. Between the sector and
/// this station its offset on each arm is linear in station.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoundaryTaper {
    /// Station on both arms at which the taper ends.
    pub station_m: f64,
    /// The `from_arm`'s own signed offset of the boundary (left, positive).
    pub from_offset_m: f64,
    /// The `to_arm`'s own signed offset of the boundary (right, negative).
    pub to_offset_m: f64,
}

/// Origin of one carriageway core-loop vertex.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoreVertexSource {
    /// A carriageway-edge sample of a sector, by index into [`JunctionLayout::samples`].
    Sample(usize),
    /// A carriageway-edge point at an arm knot between its sector station and its mouth, on the
    /// boundary's run (`SectorBoundary::run_point`).
    Knot {
        /// Sorted arm index.
        arm: usize,
        /// Arm side.
        side: Side,
        /// Knot index on the arm.
        knot: usize,
    },
    /// A carriageway-edge point at the end of an arm's width-change taper (D4), between its
    /// sector station and its mouth.
    TaperEnd {
        /// Sorted arm index.
        arm: usize,
        /// Arm side.
        side: Side,
    },
    /// A carriageway-edge point of an arm's mouth section.
    Mouth {
        /// Sorted arm index.
        arm: usize,
        /// Arm side.
        side: Side,
    },
}

/// One carriageway core-loop vertex.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoreVertex {
    /// Position in world XZ, copied from its source.
    pub position: RoadVec2,
    /// Where the position comes from.
    pub source: CoreVertexSource,
}

/// Why a node has no layout. Every variant is a placement rejection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutError {
    /// No arms were given.
    NoArms,
    /// The node position is not finite.
    NonFiniteNode,
    /// An arm holds a non-finite value.
    NonFinite {
        /// Offending arm.
        arm: ArmKey,
    },
    /// Two arms share one key.
    DuplicateArm {
        /// Repeated key.
        arm: ArmKey,
    },
    /// An arm's centreline, stations, lateral axes or `max_station_m` are malformed, or it is
    /// too short to define a direction.
    DegenerateArm {
        /// Offending arm.
        arm: ArmKey,
    },
    /// An arm side has no core band or its band offsets do not increase outward.
    InvalidBands {
        /// Offending arm.
        arm: ArmKey,
    },
    /// Two consecutive arms point in practically the same direction.
    DuplicateDirection {
        /// The later arm of the pair in counter-clockwise order.
        arm: ArmKey,
    },
    /// Matched bands across a sector differ in kind or level.
    BandMismatch {
        /// Arm whose left side bounds the sector.
        arm: ArmKey,
    },
    /// The arm cannot hold its minimum node extent, its width-change taper (D4) and its crosswalk
    /// within `max_station_m`.
    ArmTooShort {
        /// Offending arm.
        arm: ArmKey,
    },
    /// A corner does not fit within `max_station_m` even at its minimum radius.
    CornerDoesNotFit {
        /// Arm whose station limit the corner exceeds.
        arm: ArmKey,
    },
    /// The carriageway core loop touches or crosses itself.
    CoreNotSimple,
    /// An attached arm's edge does not meet its sector's outer boundary inside the node, or its
    /// ribbon outline is not simple.
    FootpathDoesNotJoin {
        /// Offending arm.
        arm: ArmKey,
    },
}

impl SectorBoundary {
    /// Where this boundary's run along its `from_arm` (`Side::Left`) or `to_arm` (`Side::Right`)
    /// leaves the sector: its station and signed offset there. The one side-to-arm mapping of
    /// every run.
    pub fn run_start(&self, side: Side) -> (f64, f64) {
        match side {
            Side::Left => (self.from_station_m, self.from_offset_m),
            Side::Right => (self.to_station_m, self.to_offset_m),
        }
    }

    /// Signed offset of the run on `side` at `station_m`, at or beyond [`Self::run_start`]: the
    /// sector's offset where the run leaves the sector, linear to the arm's own offset at the
    /// taper station, and that own offset from there out. Node pieces, the core loop and
    /// walkways all place the run with this one function.
    pub fn run_offset_m(&self, side: Side, station_m: f64) -> f64 {
        let (start_m, start_offset_m) = self.run_start(side);
        debug_assert!(station_m >= start_m, "a run starts at its sector");
        let Some(taper) = self.taper else {
            return start_offset_m;
        };
        let own_offset_m = match side {
            Side::Left => taper.from_offset_m,
            Side::Right => taper.to_offset_m,
        };
        if station_m >= taper.station_m {
            return own_offset_m;
        }
        let t = (station_m - start_m) / (taper.station_m - start_m);
        lerp_scalar(start_offset_m, own_offset_m, t)
    }

    /// Station at which the run on `side` reaches its arm's own offset: the taper station, else
    /// where it leaves the sector.
    pub fn reach_m(&self, side: Side) -> f64 {
        self.taper
            .map_or(self.run_start(side).0, |taper| taper.station_m)
    }

    /// The run's point at `station_m` on `arm`, the boundary's `from_arm` or `to_arm`.
    pub fn run_point(&self, arm: &ArmInput<'_>, side: Side, station_m: f64) -> RoadVec2 {
        arm.offset_point(station_m, self.run_offset_m(side, station_m))
    }
}

/// Calls `visit` with each station strictly between `from_m` and `to_m` at which a boundary run
/// on `arm` has a vertex: the arm's knots and the taper station, ascending, each knot with its
/// index. Straight arms make tapered runs exact lines between these stations.
pub(super) fn for_each_run_station(
    arm: &ArmInput<'_>,
    from_m: f64,
    to_m: f64,
    taper_m: Option<f64>,
    mut visit: impl FnMut(f64, Option<usize>),
) {
    let mut taper_m = taper_m.filter(|&station_m| station_m > from_m && station_m < to_m);
    for knot in arm.knots_between(from_m, to_m) {
        let station_m = arm.knots[knot].station_m;
        if let Some(taper) = taper_m.filter(|&taper| taper <= station_m) {
            if taper < station_m {
                visit(taper, None);
            }
            taper_m = None;
        }
        visit(station_m, Some(knot));
    }
    if let Some(taper) = taper_m {
        visit(taper, None);
    }
}

/// The layout of one node: sorted arms, sectors with sampled boundaries, and the carriageway
/// core loop. It is data only; node pieces and meshes are built from it.
#[derive(Clone, Debug, PartialEq)]
pub struct JunctionLayout {
    /// Node position in world XZ.
    pub node: RoadVec2,
    /// Degree class.
    pub kind: NodeKind,
    /// Arms in counter-clockwise order.
    pub arms: Vec<ArmLayout>,
    /// Sector `i` runs from arm `i` to arm `(i + 1) % k`.
    pub sectors: Vec<SectorLayout>,
    /// Sector boundaries, grouped by sector.
    pub boundaries: Vec<SectorBoundary>,
    /// Boundary samples, grouped by boundary.
    pub samples: Vec<RoadVec2>,
    /// Counter-clockwise carriageway core loop, without repeated consecutive vertices.
    pub core: Vec<CoreVertex>,
    /// Attached arms by sector and, within one, along its outer path from its from-arm.
    pub attachments: Vec<Attachment>,
}

// The working buffers of a layout build, reused between builds (`ROAD-68-K18`): placement
// validation lays out every affected node on every preview pose. Each step clears what it uses,
// so a layout does not depend on the buffers' history.
#[derive(Default)]
pub(super) struct LayoutScratch {
    // The attached arms, in sorted order, before they are placed.
    attached: Vec<ArmLayout>,
    // One sector's (segment, fraction, attachment, edge, attached station), in boundary order.
    inserts: Vec<(usize, f64, usize, usize, f64)>,
    // One sector's outermost samples, where unplaced attached edges meet them.
    outer: Vec<RoadVec2>,
    // One boundary's emitted samples, before the inserts go in.
    emitted: Vec<RoadVec2>,
    // One carriageway-edge run of the core loop.
    run: Vec<CoreVertex>,
    // Core loop pieces by first vertex.
    pieces: Vec<usize>,
    // A sector's outer path and an attached arm's ribbon.
    path: Vec<PathPoint>,
    ring: Vec<PathPoint>,
    // Per attached arm: its sector, the path positions of its two edge points, and its index.
    spans: Vec<(usize, usize, usize, usize)>,
    attachments: Vec<Attachment>,
}

impl ArmKnot {
    fn offset(&self, lateral_m: f64) -> RoadVec2 {
        self.position + self.lateral * lateral_m
    }
}

/// An arm's cross-section at one station: the straight line between the offsets of the two
/// knots around it, interpolated at the station's fraction. Mouth sections lie on it, and vehicle
/// lanes end on it (`TRAFFIC-12`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SectionLine {
    knots: [ArmKnot; 2],
    t: f64,
}

impl SectionLine {
    /// The point at signed lateral offset `lateral_m`, bitwise [`ArmInput::offset_point`]'s.
    pub fn point(&self, lateral_m: f64) -> RoadVec2 {
        let [start, end] = &self.knots;
        start.offset(lateral_m).lerp(end.offset(lateral_m), self.t)
    }

    /// Unit direction away from the node, square to the line: the interpolated lateral axis
    /// turned a quarter right.
    pub fn direction(&self) -> RoadVec2 {
        let [start, end] = &self.knots;
        let lateral = start.lateral.lerp(end.lateral, self.t);
        RoadVec2::new(lateral.y, -lateral.x).normalize()
    }
}

impl ArmSide<'_> {
    // Unsigned offset of the carriageway edge.
    pub(super) fn core_offset(&self) -> f64 {
        self.bands[self.core_bands - 1].outer_m
    }

    fn raised_width(&self) -> f64 {
        self.bands[self.bands.len() - 1].outer_m - self.core_offset()
    }

    fn beyond_core(&self) -> &[ArmBand] {
        &self.bands[self.core_bands..]
    }

    fn is_valid(&self) -> bool {
        (1..=self.bands.len()).contains(&self.core_bands)
            && self.bands[0].outer_m > 0.0
            && self
                .bands
                .windows(2)
                .all(|pair| pair[1].outer_m > pair[0].outer_m)
    }
}

impl ArmInput<'_> {
    /// Evaluates the offset curve `o(station_m, lateral_m)`: the station's point between its two
    /// knots, offset along the interpolated lateral axes. At a knot station it returns that
    /// knot's offset point exactly. Mouth sections and node pieces must use this one function.
    pub fn offset_point(&self, station_m: f64, lateral_m: f64) -> RoadVec2 {
        self.section_line(station_m).point(lateral_m)
    }

    /// The arm's cross-section line at `station_m`, on which [`Self::offset_point`] places every
    /// lateral offset of that station.
    pub fn section_line(&self, station_m: f64) -> SectionLine {
        let last_segment = self.knots.len() - 2;
        let segment = self.knots[1..]
            .partition_point(|knot| knot.station_m < station_m)
            .min(last_segment);
        let (start, end) = (self.knots[segment], self.knots[segment + 1]);
        let t = (station_m - start.station_m) / (end.station_m - start.station_m);
        SectionLine {
            knots: [start, end],
            t,
        }
    }

    // Segments whose start station lies below `station_m`.
    fn segments_below(&self, station_m: f64) -> usize {
        self.knots
            .partition_point(|knot| knot.station_m < station_m)
            .min(self.knots.len() - 1)
    }

    // Knots strictly between two stations. Never the node's own knot: a run that leaves a
    // backward miter (station < 0) passes it on the same straight first segment.
    pub(super) fn knots_between(&self, from_m: f64, to_m: f64) -> Range<usize> {
        let start = self
            .knots
            .partition_point(|knot| knot.station_m <= from_m)
            .max(1);
        let end = self.knots.partition_point(|knot| knot.station_m < to_m);
        start..end.max(start)
    }

    fn crosswalk_depth(&self) -> f64 {
        if self.crossable {
            CROSSWALK_DEPTH_M
        } else {
            0.0
        }
    }

    fn validate(&self, node: RoadVec2) -> Result<(), LayoutError> {
        let arm = self.key;
        let sides = [self.left, self.right];
        let finite = self.max_station_m.is_finite()
            && self.knots.iter().all(|knot| {
                knot.position.is_finite() && knot.lateral.is_finite() && knot.station_m.is_finite()
            })
            && sides
                .iter()
                .all(|side| side.bands.iter().all(|band| band.outer_m.is_finite()));
        if !finite {
            return Err(LayoutError::NonFinite { arm });
        }
        if !self.centreline_is_valid(node) {
            return Err(LayoutError::DegenerateArm { arm });
        }
        if !sides.iter().all(ArmSide::is_valid) {
            return Err(LayoutError::InvalidBands { arm });
        }
        Ok(())
    }

    fn centreline_is_valid(&self, node: RoadVec2) -> bool {
        let knots = self.knots;
        if knots.len() < 2 || knots[0].position != node || knots[0].station_m != 0.0 {
            return false;
        }
        let increasing = knots.windows(2).all(|pair| {
            pair[1].station_m > pair[0].station_m && pair[1].position != pair[0].position
        });
        // Each lateral axis is unit length and lies left of the segment leaving its knot (the
        // last knot uses the segment arriving).
        let lateral_ok = knots.iter().enumerate().all(|(index, knot)| {
            let along = if index + 1 < knots.len() {
                knots[index + 1].position - knot.position
            } else {
                knot.position - knots[index - 1].position
            };
            (knot.lateral.length_squared() - 1.0).abs() <= LATERAL_UNIT_TOLERANCE
                && along.perp_dot(knot.lateral) > 0.0
        });
        increasing
            && lateral_ok
            && self.max_station_m > 0.0
            && self.max_station_m <= knots[knots.len() - 1].station_m
    }

    // Unit direction over the first metre of the centreline, as `validate.rs` measures it.
    fn direction(&self) -> Option<RoadVec2> {
        let origin = self.knots[0].position;
        let mut end = origin;
        let mut remaining_m = ARM_DIRECTION_RUN_M;
        for knot in &self.knots[1..] {
            let delta = knot.position - end;
            let run_m = delta.length();
            if run_m >= remaining_m {
                end += delta * (remaining_m / run_m);
                break;
            }
            remaining_m -= run_m;
            end = knot.position;
        }
        let run = end - origin;
        let run_m = run.length();
        (run_m >= MIN_DIRECTION_RUN_M).then(|| run / run_m)
    }
}

impl JunctionLayout {
    /// Builds the layout of the node at `node` from its incident arms, in any order.
    ///
    /// The result does not depend on the order of `arms`. Degenerate input and constructions that
    /// cannot satisfy the layout rules return a [`LayoutError`]; nothing is repaired.
    pub fn build(node: RoadVec2, arms: &[ArmInput<'_>]) -> Result<Self, LayoutError> {
        let mut layout = Self::empty();
        layout.rebuild(node, arms, &mut LayoutScratch::default())?;
        Ok(layout)
    }

    // A layout without arms, for `rebuild` to fill.
    pub(super) fn empty() -> Self {
        Self {
            node: RoadVec2::ZERO,
            kind: NodeKind::Terminal,
            arms: Vec::new(),
            sectors: Vec::new(),
            boundaries: Vec::new(),
            samples: Vec::new(),
            core: Vec::new(),
            attachments: Vec::new(),
        }
    }

    // `build` in place, in this layout's vectors and `scratch`'s buffers. On success the layout
    // equals what `build` returns; on failure it returns the same error and holds a partial
    // layout, which no caller reads.
    pub(super) fn rebuild(
        &mut self,
        node: RoadVec2,
        arms: &[ArmInput<'_>],
        scratch: &mut LayoutScratch,
    ) -> Result<(), LayoutError> {
        if !node.is_finite() {
            return Err(LayoutError::NonFiniteNode);
        }
        if arms.is_empty() {
            return Err(LayoutError::NoArms);
        }
        self.node = node;
        sort_arms(node, arms, &mut self.arms)?;
        // The attached arms leave the sorted list, both keeping their order.
        let is_attached = |arm: &ArmLayout| arms[arm.input_index].attached;
        scratch.attached.clear();
        let attached = self.arms.iter().filter(|arm| is_attached(arm)).cloned();
        scratch.attached.extend(attached);
        self.arms.retain(|arm| !is_attached(arm));
        if self.arms.is_empty() {
            return Err(LayoutError::NoArms);
        }
        let (sorted, sectors, boundaries) = (&self.arms, &mut self.sectors, &mut self.boundaries);
        plan_sectors(node, arms, sorted, sectors, boundaries)?;
        let (sectors, boundaries) = (&self.sectors, &self.boundaries);
        let placed = &mut self.attachments;
        attach_to_runs(arms, sorted, sectors, boundaries, &scratch.attached, placed)?;
        self.kind = match self.arms.len() {
            1 => NodeKind::Terminal,
            2 => NodeKind::Bend,
            _ => NodeKind::Junction,
        };
        self.samples.clear();
        self.core.clear();
        assign_mouths(arms, self)?;
        sample_sectors(arms, self, scratch)?;
        core_loop(arms, self, scratch)?;
        self.finish_attachments(arms, scratch)?;
        Ok(())
    }

    /// The run of `boundary` (the `index`-th of its sector) along `side` of arm `arm` (sorted
    /// index), from the sector to the mouth, at the run's offsets (D4): its sector endpoint
    /// sample, the knots and taper station strictly between, the attached edges' points on that
    /// side beyond the carriageway edge, and the mouth corner.
    pub(super) fn side_run(
        &self,
        input: &ArmInput<'_>,
        (arm, side): (usize, Side),
        (boundary, index): (&SectorBoundary, usize),
        out: &mut Vec<PathPoint>,
    ) {
        let samples = &self.samples[boundary.samples.clone()];
        let start = match side {
            Side::Left => samples[0],
            Side::Right => samples[samples.len() - 1],
        };
        let station_m = boundary.run_start(side).0;
        let mouth_m = self.arms[arm].mouth_station_m;
        let first = out.len();
        out.push(PathPoint::at(station_m, start));
        let point =
            |station_m: f64| PathPoint::at(station_m, boundary.run_point(input, side, station_m));
        let taper_m = boundary.taper.map(|taper| taper.station_m);
        for_each_run_station(input, station_m, mouth_m, taper_m, |station_m, _| {
            out.push(point(station_m));
        });
        if index > 0 {
            let meetings = run_meetings(&self.attachments, arm).filter(|&(on, _)| on == side);
            out.extend(meetings.map(|(_, station_m)| point(station_m)));
            out[first + 1..].sort_by(|a, b| a.station_m.total_cmp(&b.station_m));
        }
        let offset_m = boundary.run_offset_m(side, mouth_m);
        out.push(PathPoint {
            station_m: mouth_m,
            xz: input.offset_point(mouth_m, offset_m),
            mouth: Some((self.arms[arm].input_index, offset_m)),
        });
    }

    /// Sector `sector`'s outermost boundary from its from-arm's left mouth corner back along that
    /// arm's run, across the sector's samples and out along its to-arm's right run to the mouth
    /// corner, without repeated points: the outer chain's order, with the road on the left.
    pub(super) fn outer_path(
        &self,
        arms: &[ArmInput<'_>],
        sector: usize,
        out: &mut Vec<PathPoint>,
    ) {
        let layout = &self.sectors[sector];
        let index = layout.boundaries.len() - 1;
        let outer = &self.boundaries[layout.boundaries.end - 1];
        out.clear();
        let input = |arm: usize| &arms[self.arms[arm].input_index];
        let (from, to) = (layout.from_arm, layout.to_arm);
        self.side_run(input(from), (from, Side::Left), (outer, index), out);
        out.reverse();
        let samples = &self.samples[outer.samples.clone()];
        out.extend(samples.iter().map(|&xz| PathPoint::at(0.0, xz)));
        self.side_run(input(to), (to, Side::Right), (outer, index), out);
        out.dedup_by(|next, previous| next.xz == previous.xz);
    }

    /// Attached arm `index`'s edge on `side`, from its boundary point out to its mouth corner.
    pub(super) fn attached_run(
        &self,
        input: &ArmInput<'_>,
        index: usize,
        side: Side,
        out: &mut Vec<PathPoint>,
    ) {
        let attachment = &self.attachments[index];
        let edge = attachment.edges[usize::from(side == Side::Left)];
        let mouth_m = attachment.mouth_station_m;
        let point =
            |station_m: f64| PathPoint::at(station_m, input.offset_point(station_m, edge.offset_m));
        out.push(PathPoint::at(edge.station_m, edge.point));
        for_each_run_station(input, edge.station_m, mouth_m, None, |station_m, _| {
            out.push(point(station_m));
        });
        out.push(PathPoint {
            mouth: Some((attachment.input_index, edge.offset_m)),
            ..point(mouth_m)
        });
    }

    /// The ribbon outline of attached arm `index`: from its right edge point along the outer
    /// path (`path`, its sector's) to its left edge point, out along its left edge to the mouth,
    /// and back along its right edge, and where the two points lie on the path. `None` when the
    /// path does not hold both points in order.
    pub(super) fn ribbon(
        &self,
        input: &ArmInput<'_>,
        index: usize,
        path: &[PathPoint],
        out: &mut Vec<PathPoint>,
    ) -> Option<(usize, usize)> {
        let [right, left] = self.attachments[index].edges.map(|edge| edge.point);
        let start = path.iter().position(|point| point.xz == right)?;
        let end = start + path[start..].iter().position(|point| point.xz == left)?;
        out.clear();
        out.extend_from_slice(&path[start..=end]);
        self.attached_run(input, index, Side::Left, out);
        let turn = out.len();
        self.attached_run(input, index, Side::Right, out);
        out[turn..].reverse();
        out.pop();
        out.dedup_by(|next, previous| next.xz == previous.xz);
        Some((start, end))
    }

    // Mouths of the attached arms, beyond their farther edge point; their ribbon outlines must
    // be simple. The arms are then ordered by sector and along its outer path, where the ribbons
    // of one sector must follow each other without touching.
    fn finish_attachments(
        &mut self,
        arms: &[ArmInput<'_>],
        scratch: &mut LayoutScratch,
    ) -> Result<(), LayoutError> {
        let LayoutScratch {
            path,
            ring,
            spans,
            attachments,
            ..
        } = scratch;
        spans.clear();
        let mut path_sector = None;
        for index in 0..self.attachments.len() {
            let attachment = &mut self.attachments[index];
            let input = &arms[attachment.input_index];
            let arm = attachment.key;
            let reach_m = attachment.edges[0]
                .station_m
                .max(attachment.edges[1].station_m);
            attachment.mouth_station_m = reach_m + MIN_MOUTH_STATION_M;
            if attachment.mouth_station_m > input.max_station_m {
                return Err(LayoutError::ArmTooShort { arm });
            }
            let sector = attachment.sector;
            if path_sector != Some(sector) {
                self.outer_path(arms, sector, path);
                path_sector = Some(sector);
            }
            let (start, end) = self
                .ribbon(input, index, path, ring)
                .ok_or(LayoutError::FootpathDoesNotJoin { arm })?;
            spans.push((sector, start, end, index));
            if !loop_is_simple(ring, &[0], |point| point.xz)
                || signed_area(ring, |point| point.xz) == 0.0
            {
                return Err(LayoutError::FootpathDoesNotJoin { arm });
            }
        }
        // A terminal's sector, and any sector that wraps through angle zero, holds its arms out
        // of angle order, so the path order is taken from the path itself. The sort is stable:
        // ties keep attachment order.
        spans.sort_by_key(|&(sector, start, _, _)| (sector, start));
        for pair in spans.windows(2) {
            let ((sector, _, end, _), (next_sector, start, _, next)) = (pair[0], pair[1]);
            if sector == next_sector && start <= end {
                let arm = self.attachments[next].key;
                return Err(LayoutError::FootpathDoesNotJoin { arm });
            }
        }
        attachments.clear();
        let ordered = spans
            .iter()
            .map(|&(.., index)| self.attachments[index].clone());
        attachments.extend(ordered);
        std::mem::swap(&mut self.attachments, attachments);
        Ok(())
    }
}

// Validates every arm and lists them in `sorted`, counter-clockwise, ties by key.
fn sort_arms(
    node: RoadVec2,
    arms: &[ArmInput<'_>],
    sorted: &mut Vec<ArmLayout>,
) -> Result<(), LayoutError> {
    sorted.clear();
    sorted.reserve(arms.len());
    for (input_index, arm) in arms.iter().enumerate() {
        arm.validate(node)?;
        // Degree is a handful of arms; the pairwise key check is cheaper than any index.
        if arms[..input_index].iter().any(|other| other.key == arm.key) {
            return Err(LayoutError::DuplicateArm { arm: arm.key });
        }
        let direction = arm
            .direction()
            .ok_or(LayoutError::DegenerateArm { arm: arm.key })?;
        sorted.push(ArmLayout {
            key: arm.key,
            input_index,
            direction,
            angle_rad: direction.y.atan2(direction.x).rem_euclid(TAU),
            mouth_station_m: 0.0,
            crosswalk: None,
            left_corner: None,
            right_corner: None,
        });
    }
    sorted.sort_by(|a, b| a.angle_rad.total_cmp(&b.angle_rad).then(a.key.cmp(&b.key)));
    Ok(())
}

// A crossing of two offset curves at stations `a_station` and `b_station`.
#[derive(Clone, Copy)]
struct Crossing {
    a_station: f64,
    b_station: f64,
    point: RoadVec2,
}

impl Crossing {
    fn key(&self) -> f64 {
        self.a_station.max(self.b_station)
    }
}

// Classifies every sector into `sectors` and records its boundary stations in `boundaries`;
// concave corners are fitted here.
fn plan_sectors(
    node: RoadVec2,
    arms: &[ArmInput<'_>],
    sorted: &[ArmLayout],
    sectors: &mut Vec<SectorLayout>,
    boundaries: &mut Vec<SectorBoundary>,
) -> Result<(), LayoutError> {
    let count = sorted.len();
    for arm in sorted {
        let input = &arms[arm.input_index];
        if MIN_MOUTH_STATION_M + input.crosswalk_depth() > input.max_station_m {
            return Err(LayoutError::ArmTooShort { arm: arm.key });
        }
    }
    sectors.clear();
    sectors.reserve(count);
    boundaries.clear();
    for from_arm in 0..count {
        let to_arm = (from_arm + 1) % count;
        let angle_rad = match count {
            1 => TAU,
            _ if to_arm == 0 => sorted[0].angle_rad + TAU - sorted[from_arm].angle_rad,
            _ => sorted[to_arm].angle_rad - sorted[from_arm].angle_rad,
        };
        if angle_rad < MIN_SECTOR_ANGLE_RAD {
            return Err(LayoutError::DuplicateDirection {
                arm: sorted[to_arm].key,
            });
        }
        let a = &arms[sorted[from_arm].input_index];
        let b = &arms[sorted[to_arm].input_index];
        if !bands_match(a.left, b.right) {
            return Err(LayoutError::BandMismatch { arm: a.key });
        }
        let kind = if count > 1 && angle_rad < PI - STRAIGHT_SECTOR_EPS_RAD {
            SectorKind::Concave
        } else if (angle_rad - PI).abs() <= STRAIGHT_SECTOR_EPS_RAD {
            SectorKind::Straight
        } else {
            SectorKind::Convex
        };
        let start = boundaries.len();
        let corner = match kind {
            SectorKind::Concave => {
                let (radius_m, crossing) = fit_corner(a, b)?;
                push_boundaries(boundaries, a, b, None, |_, _| {
                    Some((crossing.a_station, crossing.b_station))
                })?;
                Some(Corner {
                    radius_m,
                    centre: crossing.point,
                })
            }
            SectorKind::Straight => {
                // Bisects the two station-zero lateral lines; it points into the sector.
                let bisector = (a.knots[0].lateral - b.knots[0].lateral).normalize();
                let taper_m = straight_taper_m(a, b)?;
                push_boundaries(boundaries, a, b, taper_m, |from_offset, to_offset| {
                    Some((
                        bisector_station(a, from_offset, node, bisector, 1.0)?,
                        bisector_station(b, to_offset, node, bisector, -1.0)?,
                    ))
                })?;
                None
            }
            SectorKind::Convex => {
                push_boundaries(boundaries, a, b, None, |_, _| Some((0.0, 0.0)))?;
                None
            }
        };
        sectors.push(SectorLayout {
            kind,
            from_arm,
            to_arm,
            angle_rad,
            corner,
            boundaries: start..boundaries.len(),
        });
    }
    Ok(())
}

// Bands beyond the carriageway edge match by position; the core bands must agree too.
fn bands_match(a: ArmSide<'_>, b: ArmSide<'_>) -> bool {
    let same = |x: &ArmBand, y: &ArmBand| x.kind == y.kind && x.level == y.level;
    same(&a.bands[a.core_bands - 1], &b.bands[b.core_bands - 1])
        && a.beyond_core()
            .iter()
            .zip(b.beyond_core())
            .all(|(x, y)| same(x, y))
}

// Signed offsets of boundary `boundary` (from the carriageway edge outward) on `a`'s left and
// `b`'s right side. A band missing on one side keeps that side's last offset: zero width there.
fn boundary_offsets(a: &ArmInput<'_>, b: &ArmInput<'_>, boundary: usize) -> (f64, f64) {
    let offset = |side: ArmSide<'_>| {
        side.bands[(side.core_bands - 1 + boundary).min(side.bands.len() - 1)].outer_m
    };
    (offset(a.left), -offset(b.right))
}

fn boundary_count(a: &ArmInput<'_>, b: &ArmInput<'_>) -> usize {
    1 + a.left.beyond_core().len().max(b.right.beyond_core().len())
}

// D4: the taper station of a straight sector whose boundaries change offset, `None` when every
// boundary keeps its offset. The legacy span taper (`clamp(3 * width change, 12 m, 28 m)`, for a
// width change of twice this side's carriageway-edge step) is split evenly between the two
// approaches and shortened to what both arms hold beside their crosswalks. A taper shorter than
// the legacy minimum does not fit: `ArmTooShort` on the arm with the shorter budget.
fn straight_taper_m(a: &ArmInput<'_>, b: &ArmInput<'_>) -> Result<Option<f64>, LayoutError> {
    let steps = (0..boundary_count(a, b)).any(|boundary| {
        let (from_offset_m, to_offset_m) = boundary_offsets(a, b, boundary);
        from_offset_m != -to_offset_m
    });
    if !steps {
        return Ok(None);
    }
    let width_change_m = 2.0 * (a.left.core_offset() - b.right.core_offset()).abs();
    let length_m = (width_change_m * f64::from(WIDTH_CHANGE_TAPER_WIDTH_FACTOR)).clamp(
        f64::from(WIDTH_CHANGE_TAPER_MIN_M),
        f64::from(WIDTH_CHANGE_TAPER_MAX_M),
    );
    let limit = |arm: &ArmInput<'_>| arm.max_station_m - arm.crosswalk_depth();
    let (a_limit, b_limit) = (limit(a), limit(b));
    let station_m = (0.5 * length_m).min(a_limit).min(b_limit);
    if 2.0 * station_m < f64::from(WIDTH_CHANGE_TAPER_MIN_M) {
        let arm = if a_limit <= b_limit { a.key } else { b.key };
        return Err(LayoutError::ArmTooShort { arm });
    }
    Ok(Some(station_m))
}

// Appends one boundary per matched band edge, from the carriageway edge outward. `stations`
// maps the boundary's signed offsets on `a` and `b` to the stations where it leaves them. With a
// straight sector's taper station, a boundary whose offsets differ leaves both arms at their mean
// and reaches their own offsets at that station (D4).
fn push_boundaries(
    boundaries: &mut Vec<SectorBoundary>,
    a: &ArmInput<'_>,
    b: &ArmInput<'_>,
    taper_m: Option<f64>,
    stations: impl Fn(f64, f64) -> Option<(f64, f64)>,
) -> Result<(), LayoutError> {
    let (a_side, b_side) = (a.left, b.right);
    let (a_beyond, b_beyond) = (a_side.beyond_core(), b_side.beyond_core());
    let band_beyond = |index: usize| {
        a_beyond
            .get(index)
            .or_else(|| b_beyond.get(index))
            .map(|band| BoundaryBand {
                kind: band.kind,
                level: band.level,
            })
    };
    let core = a_side.bands[a_side.core_bands - 1];
    let mut inner = BoundaryBand {
        kind: core.kind,
        level: core.level,
    };
    for boundary in 0..boundary_count(a, b) {
        let (own_from_m, own_to_m) = boundary_offsets(a, b, boundary);
        let taper = taper_m
            .filter(|_| own_from_m != -own_to_m)
            .map(|station_m| BoundaryTaper {
                station_m,
                from_offset_m: own_from_m,
                to_offset_m: own_to_m,
            });
        let (from_offset_m, to_offset_m) = match taper {
            Some(_) => {
                let mean_m = 0.5 * (own_from_m - own_to_m);
                (mean_m, -mean_m)
            }
            None => (own_from_m, own_to_m),
        };
        let (from_station_m, to_station_m) =
            stations(from_offset_m, to_offset_m).ok_or(LayoutError::ArmTooShort { arm: a.key })?;
        if taper.is_some_and(|taper| taper.station_m <= from_station_m.max(to_station_m)) {
            return Err(LayoutError::ArmTooShort { arm: a.key });
        }
        let outer = band_beyond(boundary);
        boundaries.push(SectorBoundary {
            inner,
            outer,
            from_station_m,
            from_offset_m,
            to_station_m,
            to_offset_m,
            taper,
            samples: 0..0,
        });
        if let Some(outer) = outer {
            inner = outer;
        }
    }
    Ok(())
}

// Fits the largest radius in `[R_min, R]` whose corner keeps both mouths within their limits.
fn fit_corner(a: &ArmInput<'_>, b: &ArmInput<'_>) -> Result<(f64, Crossing), LayoutError> {
    let (a_edge, b_edge) = (a.left.core_offset(), b.right.core_offset());
    let min_radius = a.left.raised_width().max(b.right.raised_width()) + CURB_RETURN_CLEARANCE_M;
    let max_radius = CURB_RETURN_RADIUS_M.max(min_radius);
    let a_limit = a.max_station_m - a.crosswalk_depth();
    let b_limit = b.max_station_m - b.crosswalk_depth();
    let crossing = |radius: f64| corner_crossing(a, a_edge + radius, b, -(b_edge + radius));
    let fitted = |radius: f64| {
        crossing(radius)
            .filter(|found| found.a_station <= a_limit && found.b_station <= b_limit)
            .map(|found| (radius, found))
    };
    if let Some(fit) = fitted(max_radius) {
        return Ok(fit);
    }
    let Some(mut best) = fitted(min_radius) else {
        let arm = match crossing(min_radius) {
            Some(found) if found.a_station <= a_limit => b.key,
            _ => a.key,
        };
        return Err(LayoutError::CornerDoesNotFit { arm });
    };
    let (mut fits, mut fails) = (min_radius, max_radius);
    for _ in 0..RADIUS_BISECTION_STEPS {
        let radius = 0.5 * (fits + fails);
        match fitted(radius) {
            Some(fit) => {
                fits = radius;
                best = fit;
            }
            None => fails = radius,
        }
    }
    Ok(best)
}

// The corner centre: the first crossing at stations `>= 0`, else the first one that uses an arm's
// first offset segment extended backwards behind the node (`ROAD-68` open question 6).
fn corner_crossing(
    a: &ArmInput<'_>,
    a_offset: f64,
    b: &ArmInput<'_>,
    b_offset: f64,
) -> Option<Crossing> {
    forward_crossing(a, a_offset, b, b_offset, 0.0)
        .or_else(|| backward_crossing(a, a_offset, b, b_offset))
}

// First crossing, by `max(sa, sb)` then search order, of `o_a(., a_offset)` and
// `o_b(., b_offset)` over stations from `a_from_m` on `a` and up to each arm's `max_station_m`.
// Segment pairs are tested in station order and the search stops once no untested pair can start
// before the best crossing.
fn forward_crossing(
    a: &ArmInput<'_>,
    a_offset: f64,
    b: &ArmInput<'_>,
    b_offset: f64,
    a_from_m: f64,
) -> Option<Crossing> {
    let mut best: Option<Crossing> = None;
    let b_segments = b.segments_below(b.max_station_m);
    for a_segment in 0..a.segments_below(a.max_station_m) {
        let (a0, a1) = (&a.knots[a_segment], &a.knots[a_segment + 1]);
        if best.is_some_and(|found| found.key() <= a0.station_m) {
            break;
        }
        let (p0, p1) = (a0.offset(a_offset), a1.offset(a_offset));
        for b_segment in 0..b_segments {
            let (b0, b1) = (&b.knots[b_segment], &b.knots[b_segment + 1]);
            if best.is_some_and(|found| found.key() <= a0.station_m.max(b0.station_m)) {
                break;
            }
            let Some((t, u)) = segment_crossing(p0, p1, b0.offset(b_offset), b1.offset(b_offset))
            else {
                continue;
            };
            let found = Crossing {
                a_station: lerp_scalar(a0.station_m, a1.station_m, t),
                b_station: lerp_scalar(b0.station_m, b1.station_m, u),
                point: p0.lerp(p1, t),
            };
            if (a_from_m..=a.max_station_m).contains(&found.a_station)
                && found.b_station <= b.max_station_m
                && best.is_none_or(|current| found.key() < current.key())
            {
                best = Some(found);
            }
        }
    }
    best
}

// First crossing, by `max(sa, sb)` then search order, in which at least one curve runs on its
// first offset segment extended behind the node (station < 0, at most `max_station_m` back). An
// obtuse sector between a narrower and a wider carriageway has its corner centre there: on the
// wider arm's curb-offset line just behind its station zero. Only pairs with a first segment can
// cross backwards, so this tests `O(Pa + Pb)` pairs.
fn backward_crossing(
    a: &ArmInput<'_>,
    a_offset: f64,
    b: &ArmInput<'_>,
    b_offset: f64,
) -> Option<Crossing> {
    let (a_segments, b_segments) = (
        a.segments_below(a.max_station_m),
        b.segments_below(b.max_station_m),
    );
    let pairs = (0..b_segments)
        .map(|b_segment| (0, b_segment))
        .chain((1..a_segments).map(|a_segment| (a_segment, 0)));
    let mut best: Option<Crossing> = None;
    for (a_segment, b_segment) in pairs {
        let (a0, a1) = (&a.knots[a_segment], &a.knots[a_segment + 1]);
        let (b0, b1) = (&b.knots[b_segment], &b.knots[b_segment + 1]);
        let (p0, p1) = (a0.offset(a_offset), a1.offset(a_offset));
        let Some((t, u)) = line_crossing(p0, p1, b0.offset(b_offset), b1.offset(b_offset)) else {
            continue;
        };
        let along = |parameter: f64, first: bool| {
            (0.0..=1.0).contains(&parameter) || (first && parameter < 0.0)
        };
        if !along(t, a_segment == 0) || !along(u, b_segment == 0) {
            continue;
        }
        let found = Crossing {
            a_station: lerp_scalar(a0.station_m, a1.station_m, t),
            b_station: lerp_scalar(b0.station_m, b1.station_m, u),
            point: p0.lerp(p1, t),
        };
        let within = |station: f64, arm: &ArmInput<'_>| station.abs() <= arm.max_station_m;
        if within(found.a_station, a)
            && within(found.b_station, b)
            && best.is_none_or(|current| found.key() < current.key())
        {
            best = Some(found);
        }
    }
    best
}

// Station at which `o(., lateral_m)` stops lying past the sector bisector. `past_sign` is +1 for
// the sector's `from_arm` (past means counter-clockwise of the bisector) and -1 for its `to_arm`.
// When the station-zero point is not past it (a sector at least a half turn wide), the first
// offset segment extended behind the node reaches the bisector at a station `<= 0`, so both arms
// still meet in one miter point rather than a rounding-sized arc. `None` if the curve never
// clears the bisector.
fn bisector_station(
    arm: &ArmInput<'_>,
    lateral_m: f64,
    node: RoadVec2,
    bisector: RoadVec2,
    past_sign: f64,
) -> Option<f64> {
    let past = |knot: &ArmKnot| past_sign * bisector.perp_dot(knot.offset(lateral_m) - node);
    let mut previous = past(&arm.knots[0]);
    if previous <= 0.0 {
        // A straight sector's bisector lies within half a degree of the station-zero lateral
        // line, so the first segment always moves away from it; `None` would mean a sector that
        // is not straight.
        let next = past(&arm.knots[1]);
        return (next < previous).then(|| arm.knots[1].station_m * previous / (previous - next));
    }
    for pair in arm.knots[..=arm.segments_below(arm.max_station_m)].windows(2) {
        let next = past(&pair[1]);
        if next <= 0.0 {
            let station_m = lerp_scalar(
                pair[0].station_m,
                pair[1].station_m,
                previous / (previous - next),
            );
            return (station_m <= arm.max_station_m).then_some(station_m);
        }
        previous = next;
    }
    None
}

// An attached edge not yet found on its sector's outermost samples.
const UNPLACED: BoundaryPoint = BoundaryPoint::Sample(usize::MAX);

// Places each attached arm in the sector it lies in and finds where its edges meet the
// outermost runs of that sector's two arm sides (the right edge tries the from-arm first, the
// left edge the to-arm). A run point lies at or beyond the run's reach, where it holds its arm's
// own offset (D4); edges that meet neither run are left `UNPLACED` for the sector's samples.
// Lists them in `placed`, in `attached`'s order, up to the first that fails.
fn attach_to_runs(
    arms: &[ArmInput<'_>],
    sorted: &[ArmLayout],
    sectors: &[SectorLayout],
    boundaries: &[SectorBoundary],
    attached: &[ArmLayout],
    placed: &mut Vec<Attachment>,
) -> Result<(), LayoutError> {
    let attach = |arm: &ArmLayout| {
        let input = &arms[arm.input_index];
        let sector = sectors
            .iter()
            .position(|sector| {
                let into = (arm.angle_rad - sorted[sector.from_arm].angle_rad).rem_euclid(TAU);
                into >= MIN_SECTOR_ANGLE_RAD && sector.angle_rad - into >= MIN_SECTOR_ANGLE_RAD
            })
            .ok_or(LayoutError::DuplicateDirection { arm: arm.key })?;
        let layout = &sectors[sector];
        let outer = &boundaries[layout.boundaries.end - 1];
        let runs = [(layout.from_arm, Side::Left), (layout.to_arm, Side::Right)];
        // The run is searched at its arm's own offset; its point is the run's own, tapered or not.
        let edge = |offset_m: f64, order: [usize; 2]| {
            let on_run = order.into_iter().find_map(|run| {
                let (road_arm, side) = runs[run];
                let road = &arms[sorted[road_arm].input_index];
                // Beyond the taper the run lies on its arm's offset curve at its own offset, so
                // the crossing found there is a point of the run itself.
                let reach_m = outer.reach_m(side);
                let road_offset_m = outer.run_offset_m(side, reach_m);
                let found = forward_crossing(road, road_offset_m, input, offset_m, reach_m)?;
                Some(AttachedEdge {
                    offset_m,
                    station_m: found.b_station,
                    point: outer.run_point(road, side, found.a_station),
                    on: BoundaryPoint::Run {
                        arm: road_arm,
                        side,
                        station_m: found.a_station,
                    },
                })
            });
            on_run.unwrap_or(AttachedEdge {
                offset_m,
                station_m: 0.0,
                point: RoadVec2::ZERO,
                on: UNPLACED,
            })
        };
        let outer_m = |side: ArmSide<'_>| side.bands[side.bands.len() - 1].outer_m;
        Ok(Attachment {
            key: arm.key,
            input_index: arm.input_index,
            direction: arm.direction,
            sector,
            mouth_station_m: 0.0,
            edges: [
                edge(-outer_m(input.right), [0, 1]),
                edge(outer_m(input.left), [1, 0]),
            ],
        })
    };
    placed.clear();
    for arm in attached {
        placed.push(attach(arm)?);
    }
    Ok(())
}

// Sets each arm's mouth and crosswalk stations and copies its corner tangents. The zebra stays
// right outside the corners; an attached edge that meets an arm side moves only the mouth, to
// `MIN_MOUTH_STATION_M` beyond that point (`ROAD-68` open question 17). Every run from where a
// boundary reaches its arm's own offset to the mouth is empty or at least `MIN_MOUTH_STATION_M`
// long: where one would be shorter, the mouth moves that far beyond the farthest reach, so two
// corners of an arm without a crossing that end a hair apart leave no sliver between the nearer
// one and the mouth (`ROAD-68-F` part 2; a crossing's strip is already longer).
fn assign_mouths(arms: &[ArmInput<'_>], layout: &mut JunctionLayout) -> Result<(), LayoutError> {
    let (sectors, boundaries, attachments) =
        (&layout.sectors, &layout.boundaries, &layout.attachments);
    let count = layout.arms.len();
    for (index, arm) in layout.arms.iter_mut().enumerate() {
        let input = &arms[arm.input_index];
        let left = &sectors[index];
        let right = &sectors[(index + count - 1) % count];
        let attached_m =
            run_meetings(attachments, index).map(|(_, station_m)| station_m + MIN_MOUTH_STATION_M);
        let reaches = || {
            boundaries[left.boundaries.clone()]
                .iter()
                .map(|boundary| boundary.reach_m(Side::Left))
                .chain(
                    boundaries[right.boundaries.clone()]
                        .iter()
                        .map(|boundary| boundary.reach_m(Side::Right)),
                )
        };
        let reach_m = reaches().fold(MIN_MOUTH_STATION_M, f64::max);
        let mouth_m = attached_m.fold(reach_m + input.crosswalk_depth(), f64::max);
        let short_run =
            reaches().any(|run_m| run_m < mouth_m && mouth_m - run_m < MIN_MOUTH_STATION_M);
        arm.mouth_station_m = if short_run {
            reaches().fold(f64::NEG_INFINITY, f64::max) + MIN_MOUTH_STATION_M
        } else {
            mouth_m
        };
        if arm.mouth_station_m > input.max_station_m {
            return Err(LayoutError::ArmTooShort { arm: arm.key });
        }
        arm.crosswalk = input.crossable.then(|| {
            let station_m = reach_m + 0.5 * CROSSWALK_DEPTH_M;
            Crosswalk {
                inner_station_m: reach_m,
                station_m,
                kerbs: [
                    input.offset_point(station_m, -input.right.core_offset()),
                    input.offset_point(station_m, input.left.core_offset()),
                ],
            }
        });
        arm.left_corner = left.corner.map(|corner| CornerTangent {
            station_m: boundaries[left.boundaries.start].from_station_m,
            radius_m: corner.radius_m,
        });
        arm.right_corner = right.corner.map(|corner| CornerTangent {
            station_m: boundaries[right.boundaries.start].to_station_m,
            radius_m: corner.radius_m,
        });
    }
    Ok(())
}

// The side and station of each attached edge that meets the outermost run of arm `arm`.
fn run_meetings(attachments: &[Attachment], arm: usize) -> impl Iterator<Item = (Side, f64)> + '_ {
    let edges = attachments.iter().flat_map(|attachment| attachment.edges);
    edges.filter_map(move |edge| match edge.on {
        BoundaryPoint::Run {
            arm: on,
            side,
            station_m,
        } if on == arm => Some((side, station_m)),
        _ => None,
    })
}

// One boundary curve in polar form about its centre, between two exact endpoints.
struct PolarCurve {
    start: RoadVec2,
    end: RoadVec2,
    centre: RoadVec2,
    start_angle: f64,
    sweep: f64,
    start_radius: f64,
    end_radius: f64,
}

impl PolarCurve {
    fn new(start: RoadVec2, end: RoadVec2, centre: RoadVec2, half_turn: bool) -> Self {
        let (from, to) = (start - centre, end - centre);
        // A terminal cap turns half a revolution counter-clockwise between antipodal endpoints;
        // every other curve takes the signed angle between its endpoints.
        let sweep = if half_turn {
            PI
        } else {
            from.perp_dot(to).atan2(from.dot(to))
        };
        Self {
            start,
            end,
            centre,
            start_angle: from.y.atan2(from.x),
            sweep,
            start_radius: from.length(),
            end_radius: to.length(),
        }
    }

    fn segment_count(&self) -> usize {
        let turn = self.sweep.abs();
        let length = turn * self.start_radius.max(self.end_radius)
            + (self.end_radius - self.start_radius).abs();
        let by_length = (length / MAX_SAMPLE_SEGMENT_M).ceil();
        let by_angle = (turn / MAX_SAMPLE_ANGLE_RAD).ceil();
        by_length.max(by_angle).max(1.0) as usize
    }

    fn emit(&self, segments: usize, samples: &mut Vec<RoadVec2>) {
        samples.push(self.start);
        // A point boundary repeats its one value rather than re-deriving it through the polar form.
        if self.start == self.end {
            samples.extend(std::iter::repeat_n(self.start, segments));
            return;
        }
        for step in 1..segments {
            let t = step as f64 / segments as f64;
            let radius = lerp_scalar(self.start_radius, self.end_radius, t);
            // A half turn's middle sample, the terminal cap's apex, is its start turned a quarter
            // counter-clockwise: exact, so an axis-aligned arm's apex lies on its axis, where the
            // polar form (`cos(pi / 2)` is not zero) put it 6e-16 m off.
            if self.sweep == PI && step * 2 == segments {
                let from = self.start - self.centre;
                samples.push(self.centre + from.perp() * (radius / self.start_radius));
                continue;
            }
            let (sin, cos) = (self.start_angle + self.sweep * t).sin_cos();
            samples.push(self.centre + RoadVec2::new(cos, sin) * radius);
        }
        samples.push(self.end);
    }
}

// Samples every sector boundary with one sample count per sector. Attached edges that met no
// run meet the sector's outermost samples: each meeting point is inserted there, and the same
// fraction of the same segment into every other boundary of the sector.
fn sample_sectors(
    arms: &[ArmInput<'_>],
    layout: &mut JunctionLayout,
    scratch: &mut LayoutScratch,
) -> Result<(), LayoutError> {
    let (node, sorted, samples) = (layout.node, &layout.arms, &mut layout.samples);
    let (sectors, boundaries) = (&mut layout.sectors, &mut layout.boundaries);
    let attachments = &mut layout.attachments;
    let LayoutScratch {
        inserts,
        outer,
        emitted,
        ..
    } = scratch;
    let half_turn = sorted.len() == 1;
    for (sector_index, sector) in sectors.iter_mut().enumerate() {
        let a = &arms[sorted[sector.from_arm].input_index];
        let b = &arms[sorted[sector.to_arm].input_index];
        let centre = sector.corner.map_or(node, |corner| corner.centre);
        let curve = |boundary: &SectorBoundary| {
            let start = a.offset_point(boundary.from_station_m, boundary.from_offset_m);
            // In a straight sector's miter every boundary has equal offsets on both arms: the
            // arms' own where no boundary steps, the mean of the two where one does (D4's taper).
            // Its two bisector crossings are then the same point, so the `from_arm` one stands
            // for both instead of leaving a rounding-sized step between two computations of it.
            let end = if sector.kind == SectorKind::Straight {
                start
            } else {
                b.offset_point(boundary.to_station_m, boundary.to_offset_m)
            };
            PolarCurve::new(start, end, centre, half_turn)
        };
        let range = sector.boundaries.clone();
        let mut segments = 1;
        for boundary in &boundaries[range.clone()] {
            let curve = curve(boundary);
            // A curb return turns clockwise about its centre; anything else cannot be built.
            if sector.kind == SectorKind::Concave && curve.sweep >= 0.0 {
                return Err(LayoutError::CornerDoesNotFit { arm: a.key });
            }
            segments = segments.max(curve.segment_count());
        }
        inserts.clear();
        outer.clear();
        for (attachment, layout) in attachments.iter().enumerate() {
            for (edge, placed) in layout.edges.iter().enumerate() {
                if layout.sector != sector_index || placed.on != UNPLACED {
                    continue;
                }
                if outer.is_empty() {
                    curve(&boundaries[range.end - 1]).emit(segments, outer);
                }
                let input = &arms[layout.input_index];
                let (segment, u, station_m) = polyline_crossing(input, placed.offset_m, outer)
                    .ok_or(LayoutError::FootpathDoesNotJoin { arm: layout.key })?;
                inserts.push((segment, u, attachment, edge, station_m));
            }
        }
        inserts.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
        let outermost = range.end - 1;
        for (index, boundary) in boundaries[range.clone()].iter_mut().enumerate() {
            let start = samples.len();
            curve(boundary).emit(segments, samples);
            if inserts.is_empty() {
                boundary.samples = start..samples.len();
                continue;
            }
            emitted.clear();
            emitted.extend(samples.drain(start..));
            let mut next = inserts.iter().peekable();
            for (i, &sample) in emitted.iter().enumerate() {
                samples.push(sample);
                while let Some(&(_, u, attachment, edge, station_m)) =
                    next.next_if(|insert| insert.0 == i)
                {
                    let point = sample.lerp(emitted[i + 1], u);
                    if range.start + index == outermost {
                        let placed = &mut attachments[attachment].edges[edge];
                        placed.station_m = station_m;
                        placed.point = point;
                        placed.on = BoundaryPoint::Sample(samples.len());
                    }
                    samples.push(point);
                }
            }
            boundary.samples = start..samples.len();
        }
    }
    Ok(())
}

// The first point, by station along `arm`'s offset curve at `offset_m`, at which that curve
// crosses the polyline `points`: its segment there, the fraction along it and the station.
fn polyline_crossing(
    arm: &ArmInput<'_>,
    offset_m: f64,
    points: &[RoadVec2],
) -> Option<(usize, f64, f64)> {
    let mut best: Option<(usize, f64, f64)> = None;
    for pair in arm.knots[..=arm.segments_below(arm.max_station_m)].windows(2) {
        let (p0, p1) = (pair[0].offset(offset_m), pair[1].offset(offset_m));
        for (segment, edge) in points.windows(2).enumerate() {
            let Some((t, u)) = segment_crossing(p0, p1, edge[0], edge[1]) else {
                continue;
            };
            let station_m = lerp_scalar(pair[0].station_m, pair[1].station_m, t);
            if station_m <= arm.max_station_m
                && best.is_none_or(|(_, _, best_m)| station_m < best_m)
            {
                best = Some((segment, u, station_m));
            }
        }
        if best.is_some() {
            break;
        }
    }
    best
}

// Builds the counter-clockwise carriageway core loop and checks that it is simple.
fn core_loop(
    arms: &[ArmInput<'_>],
    layout: &mut JunctionLayout,
    scratch: &mut LayoutScratch,
) -> Result<(), LayoutError> {
    let (sorted, sectors) = (&layout.arms, &layout.sectors);
    let (boundaries, samples) = (&layout.boundaries, &layout.samples);
    let count = sorted.len();
    // Capacity hints: a reused layout already has them, a fresh one allocates once.
    let core = &mut layout.core;
    core.clear();
    core.reserve(samples.len() + 8 * count);
    let LayoutScratch {
        run: run_scratch,
        pieces,
        ..
    } = scratch;
    // Loop pieces (each arm's edges and mouth, each sector's curve), by first vertex.
    pieces.clear();
    pieces.reserve(2 * count);
    let push = |core: &mut Vec<CoreVertex>, position: RoadVec2, source: CoreVertexSource| {
        if core.last().is_none_or(|last| last.position != position) {
            core.push(CoreVertex { position, source });
        }
    };
    for (index, arm) in sorted.iter().enumerate() {
        let input = &arms[arm.input_index];
        let left_sector = &sectors[index];
        let right_sector = &sectors[(index + count - 1) % count];
        let right_edge = &boundaries[right_sector.boundaries.start];
        let left_edge = &boundaries[left_sector.boundaries.start];
        let (right, left) = (-input.right.core_offset(), input.left.core_offset());
        let mouth = arm.mouth_station_m;
        // The carriageway-edge runs between the sectors and the mouth, as node pieces build them.
        let run = |edge: &SectorBoundary, side: Side, out: &mut Vec<CoreVertex>| {
            let start_m = edge.run_start(side).0;
            let taper_m = edge.taper.map(|taper| taper.station_m);
            for_each_run_station(input, start_m, mouth, taper_m, |station_m, knot| {
                let source = match knot {
                    Some(knot) => CoreVertexSource::Knot {
                        arm: index,
                        side,
                        knot,
                    },
                    None => CoreVertexSource::TaperEnd { arm: index, side },
                };
                let position = edge.run_point(input, side, station_m);
                out.push(CoreVertex { position, source });
            });
        };
        pieces.push(core.len());
        run_scratch.clear();
        run(right_edge, Side::Right, run_scratch);
        for vertex in run_scratch.drain(..) {
            push(core, vertex.position, vertex.source);
        }
        for (side, offset) in [(Side::Right, right), (Side::Left, left)] {
            let source = CoreVertexSource::Mouth { arm: index, side };
            push(core, input.offset_point(mouth, offset), source);
        }
        run(left_edge, Side::Left, run_scratch);
        for vertex in run_scratch.drain(..).rev() {
            push(core, vertex.position, vertex.source);
        }
        pieces.push(core.len());
        for sample in boundaries[left_sector.boundaries.start].samples.clone() {
            push(core, samples[sample], CoreVertexSource::Sample(sample));
        }
    }
    if core.len() > 1 && core[0].position == core[core.len() - 1].position {
        core.pop();
    }
    let simple = loop_is_simple(core, pieces, |vertex| vertex.position)
        && signed_area(core, |vertex| vertex.position) > 0.0;
    simple.then_some(()).ok_or(LayoutError::CoreNotSimple)
}

#[derive(Clone, Copy)]
struct Bounds {
    min: RoadVec2,
    max: RoadVec2,
}

impl Bounds {
    fn of(a: RoadVec2, b: RoadVec2) -> Self {
        Self {
            min: a.min(b),
            max: a.max(b),
        }
    }

    fn union(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    fn overlaps(&self, other: &Self) -> bool {
        self.min.x <= other.max.x
            && other.min.x <= self.max.x
            && self.min.y <= other.max.y
            && other.min.y <= self.max.y
    }
}

// No two non-adjacent segments share a point and no consecutive pair folds back on itself.
// Segment pairs are tested only between pieces whose bounds overlap.
fn loop_is_simple<T>(points: &[T], pieces: &[usize], position: impl Fn(&T) -> RoadVec2) -> bool {
    let count = points.len();
    if count < 3 {
        return false;
    }
    let point = |vertex: usize| position(&points[vertex % count]);
    let segment_bounds = |segment: usize| Bounds::of(point(segment), point(segment + 1));
    let piece_segments = |piece: usize| {
        let start = pieces[piece].min(count);
        start..pieces.get(piece + 1).map_or(count, |&end| end.min(count))
    };
    let piece_bounds = |piece: usize| {
        piece_segments(piece)
            .map(segment_bounds)
            .reduce(Bounds::union)
    };
    let folds_back = (0..count).any(|vertex| {
        let (a, b, c) = (point(vertex), point(vertex + 1), point(vertex + 2));
        orient(a, b, c) == 0.0 && (b - a).dot(c - b) < 0.0
    });
    if folds_back {
        return false;
    }
    for piece in 0..pieces.len() {
        let Some(bounds) = piece_bounds(piece) else {
            continue;
        };
        for other in piece..pieces.len() {
            if !piece_bounds(other).is_some_and(|other_bounds| bounds.overlaps(&other_bounds)) {
                continue;
            }
            for first in piece_segments(piece) {
                for second in piece_segments(other) {
                    let adjacent = second == first + 1 || (first == 0 && second == count - 1);
                    if second <= first || adjacent {
                        continue;
                    }
                    if segment_bounds(first).overlaps(&segment_bounds(second))
                        && segments_touch(
                            point(first),
                            point(first + 1),
                            point(second),
                            point(second + 1),
                        )
                    {
                        return false;
                    }
                }
            }
        }
    }
    true
}

// Exact orientation of `c` against the directed line `a -> b`: positive on its left.
fn orient(a: RoadVec2, b: RoadVec2, c: RoadVec2) -> f64 {
    let coord = |point: RoadVec2| robust::Coord {
        x: point.x,
        y: point.y,
    };
    robust::orient2d(coord(a), coord(b), coord(c))
}

// Whether two segments share any point: a proper crossing, or an endpoint on the other segment.
// Exact predicates, so the answer is the one the core triangulation will see.
fn segments_touch(p0: RoadVec2, p1: RoadVec2, q0: RoadVec2, q1: RoadVec2) -> bool {
    let (d1, d2) = (orient(q0, q1, p0), orient(q0, q1, p1));
    let (d3, d4) = (orient(p0, p1, q0), orient(p0, p1, q1));
    let straddles = |x: f64, y: f64| (x > 0.0 && y < 0.0) || (x < 0.0 && y > 0.0);
    if straddles(d1, d2) && straddles(d3, d4) {
        return true;
    }
    let within = |a: RoadVec2, b: RoadVec2, point: RoadVec2| {
        point.cmpge(a.min(b)).all() && point.cmple(a.max(b)).all()
    };
    (d1 == 0.0 && within(q0, q1, p0))
        || (d2 == 0.0 && within(q0, q1, p1))
        || (d3 == 0.0 && within(p0, p1, q0))
        || (d4 == 0.0 && within(p0, p1, q1))
}

// Parameters `(t, u)` in `[0, 1]` at which two segments cross; parallel segments never do.
pub(super) fn segment_crossing(
    p0: RoadVec2,
    p1: RoadVec2,
    q0: RoadVec2,
    q1: RoadVec2,
) -> Option<(f64, f64)> {
    line_crossing(p0, p1, q0, q1)
        .filter(|(t, u)| (0.0..=1.0).contains(t) && (0.0..=1.0).contains(u))
}

// Parameters `(t, u)` along `p0 -> p1` and `q0 -> q1` of their lines' crossing, unbounded.
fn line_crossing(p0: RoadVec2, p1: RoadVec2, q0: RoadVec2, q1: RoadVec2) -> Option<(f64, f64)> {
    let (along_p, along_q) = (p1 - p0, q1 - q0);
    let denominator = along_p.perp_dot(along_q);
    if denominator == 0.0 {
        return None;
    }
    let between = q0 - p0;
    Some((
        between.perp_dot(along_q) / denominator,
        between.perp_dot(along_p) / denominator,
    ))
}

// Shoelace area, relative to the first vertex to keep world coordinates out of the products.
fn signed_area<T>(points: &[T], position: impl Fn(&T) -> RoadVec2) -> f64 {
    let origin = position(&points[0]);
    points
        .windows(2)
        .map(|pair| (position(&pair[0]) - origin).perp_dot(position(&pair[1]) - origin))
        .sum::<f64>()
        * 0.5
}

// Exact at both ends, as glam's `lerp` for points: `t == 0` returns `a` and `t == 1` returns `b`.
fn lerp_scalar(a: f64, b: f64, t: f64) -> f64 {
    a * (1.0 - t) + b * t
}

#[cfg(test)]
mod tests;
