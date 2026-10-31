// SPDX-License-Identifier: GPL-2.0-only

//! Node arms from the graph (`ROAD-68-C`): one [`ArmInput`] per live incident edge end, oriented
//! away from the node, the rule that decides which nodes get a layout, and the D8 rule that
//! decides which arms cross ([`pass_through_crossings`] for nodes without one).
//!
//! An arm's knots are its edge's control polyline points in world XZ, walked away from the node,
//! with exact duplicates dropped: authored geometry only, so the layout, and the clips it sets,
//! never read the knots the profile solve inserts into the physical profile (`ROAD-68`
//! invariant 6). Stations are horizontal metres from the node. Heights and span stations come
//! from the physical profile, the roadbed centreline spans section, which follows the same
//! alignment with extra knots on its segments. Each knot's lateral
//! axis is the span frame's: the centred secant at interior knots and the segment direction at
//! the two ends, rotated a quarter turn to the left of the outward direction. Bands come from the
//! edge's [`RoadProfile`]: for an arm leaving from its edge's start, the layout's left is the
//! profile's positive lateral side; at the edge's end both the axis and the sides swap.
//!
//! Collection is `O(degree * arm points)`. [`NodeArms::collect`] and [`NodeArms::layout`]
//! allocate the returned buffers; collecting and laying out again in reused arms, layout and
//! `ArmsScratch` (`NodeArms::collect_into`, `NodeArms::layout_into`) allocates nothing once
//! they have grown to the node (`ROAD-68-K18`).

use super::layout::{
    ArmBand, ArmInput, ArmKey, ArmKnot, ArmSide, CROSSWALK_DEPTH_M, EdgeEnd, JunctionLayout,
    LayoutError, LayoutScratch,
};
use super::mesh::{SpanEnd, StripKind};
use super::profile::{Level, ProfileBand, RoadProfile};
use super::span::{PASS_THROUGH_DOT_THRESHOLD, edge_points, incident_direction, is_surface_edge};
use super::{BandKind, RoadVec2};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, TransitFlags, TransitType};
use godot::prelude::Vector3;
use std::ops::Range;

/// Where a mouth lies on its edge's centreline points: between `points[lo]` and `points[hi]`
/// (`lo < hi`) at fraction `t` from `lo`, so span stations can be measured the span's way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CentrelinePosition {
    pub(super) lo: usize,
    pub(super) hi: usize,
    pub(super) t: f64,
}

// One arm's slices of the shared buffers.
#[derive(Clone, Debug)]
struct ArmRecord {
    key: ArmKey,
    knots: Range<usize>,
    centreline: Range<usize>,
    // Left, then right: the side's bands and its core band count.
    sides: [(Range<usize>, usize); 2],
    max_station_m: f64,
    crossable: bool,
    attached: bool,
    // A bridge or tunnel: at a grounded node its mouth is a structure mouth (`TERRAIN-07-C`).
    structure: bool,
    // A bridge: its span hangs the abutment face below that mouth (`TERRAIN-07-C` round 4).
    bridge: bool,
}

/// The arms of one node, owning the knot and band buffers their [`ArmInput`]s borrow.
#[derive(Clone, Debug, Default)]
pub(crate) struct NodeArms {
    /// Node id.
    pub(super) node: u32,
    /// Node position in world XZ; every arm's first knot.
    pub(super) position: RoadVec2,
    arms: Vec<ArmRecord>,
    knots: Vec<ArmKnot>,
    // Per arm, its edge's physical centreline walked away from the node: horizontal station,
    // index into the edge's centreline points, and height.
    centreline: Vec<(f64, usize, f64)>,
    bands: Vec<ArmBand>,
    // A bend's default crossing that its layout cannot hold moves from the first arm (input
    // order) to the second (`ROAD-68` open question 13).
    crossing_fallback: Option<[usize; 2]>,
    // Whether a standard edge meets the node. Where only bridges and tunnels meet, the piece is
    // part of the structure and stands off the terrain (`TERRAIN-07-C`).
    grounded: bool,
}

// The buffers arms are collected and laid out in, reused between nodes (`ROAD-68-K18`).
#[derive(Default)]
pub(super) struct ArmsScratch {
    // The node's live surface edge ends, sorted by key, with their directions away from it.
    incidents: Vec<(ArmKey, RoadVec2)>,
    // The road arms' input indices and incidents.
    roads: Vec<usize>,
    road_incidents: Vec<(ArmKey, RoadVec2)>,
    // Empty between layouts: kept only for its allocation (`recycle`).
    inputs: Vec<ArmInput<'static>>,
    layout: LayoutScratch,
}

impl NodeArms {
    /// Arms of `node`, or `None` when the node has no layout in this gate: merged into another
    /// node, no surface arm, a pass-through pair (its spans share one bisected section), or a
    /// bridge or tunnel terminal. At a node with a road arm, footpath arms are attached (D5):
    /// they bound no sector and join the sidewalk ring; where only footpaths meet, they are
    /// ordinary arms. The crossing rule counts road arms only.
    pub(crate) fn collect(graph: &RegionGraph, node: u32) -> Option<Self> {
        let mut arms = Self::default();
        arms.collect_into(graph, node, &mut ArmsScratch::default())
            .then_some(arms)
    }

    /// The one arm `key` of `node` (its valid node), with the knots, laterals and centreline
    /// [`Self::collect`] gives it, whatever the node's other arms: an arm's knots depend on its
    /// own edge and the node's position only. `None` for an edge without two distinct points.
    /// `O(edge points)`.
    pub(super) fn single(graph: &RegionGraph, node: u32, key: ArmKey) -> Option<Self> {
        let pos = graph.node(graph.get_valid_node(node)).pos;
        let mut arms = Self {
            node,
            position: RoadVec2::new(f64::from(pos.x), f64::from(pos.z)),
            ..Self::default()
        };
        arms.push_arm(key, graph.edge(key.edge), false, false);
        (arms.knots.len() >= 2 && arms.centreline.len() >= 2).then_some(arms)
    }

    // `collect` in place: refills these arms with `node`'s, in their own vectors and `scratch`'s
    // incident lists, and returns false where `collect` returns `None` (the arms are then stale).
    pub(super) fn collect_into(
        &mut self,
        graph: &RegionGraph,
        node: u32,
        scratch: &mut ArmsScratch,
    ) -> bool {
        let ArmsScratch {
            incidents,
            roads,
            road_incidents,
            ..
        } = scratch;
        if !node_incidents_into(graph, node, incidents) {
            return false;
        }
        if incidents.is_empty()
            || passes_through(graph, incidents)
            || ends_a_structure(graph, incidents)
        {
            return false;
        }
        let pos = graph.node(node).pos;
        // Input indices of the road arms, which alone cross and, at a road node, bound sectors.
        roads.clear();
        roads.extend((0..incidents.len()).filter(|&index| is_road(graph, incidents[index].0)));
        road_incidents.clear();
        road_incidents.extend(roads.iter().map(|&index| incidents[index]));
        let degree_two = match road_incidents[..] {
            [a, b] => Some(degree_two_crossings(graph, node, [a, b], false)),
            _ => None,
        };
        self.node = node;
        self.position = RoadVec2::new(f64::from(pos.x), f64::from(pos.z));
        self.arms.clear();
        self.arms.reserve(incidents.len());
        self.knots.clear();
        self.centreline.clear();
        self.bands.clear();
        self.crossing_fallback = degree_two
            .as_ref()
            .and_then(|crossings| crossings.fallback)
            .map(|pair| pair.map(|road| roads[road]));
        self.grounded = incidents
            .iter()
            .any(|(key, _)| graph.edge(key.edge).class == EdgeClass::Standard);
        for (index, &(key, _)) in incidents.iter().enumerate() {
            let road = roads.iter().position(|&input| input == index);
            let crossable = road.is_some_and(|road| match &degree_two {
                Some(crossings) => crossings.crosses[road],
                None => junction_arm_crosses(graph, node, road_incidents, road),
            });
            let attached = road.is_none() && !roads.is_empty();
            self.push_arm(key, graph.edge(key.edge), crossable, attached);
        }
        true
    }

    fn push_arm(&mut self, key: ArmKey, edge: &Edge, crossable: bool, attached: bool) {
        let knots_start = self.knots.len();
        let mut station_m = 0.0;
        for (_, point) in walk_away(&edge.geometry, key.end) {
            let mut position = RoadVec2::new(f64::from(point.x), f64::from(point.z));
            if self.knots.len() == knots_start {
                position = self.position;
            } else {
                let previous = self.knots[self.knots.len() - 1].position;
                if position == previous {
                    continue;
                }
                station_m += position.distance(previous);
            }
            self.knots.push(ArmKnot {
                position,
                lateral: RoadVec2::ZERO,
                station_m,
            });
        }
        let centreline_start = self.centreline.len();
        let mut centreline_m = 0.0;
        let mut previous = None;
        for (index, point) in walk_away(edge_points(edge), key.end) {
            let position = RoadVec2::new(f64::from(point.x), f64::from(point.z));
            if let Some(previous) = previous {
                centreline_m += position.distance(previous);
            }
            previous = Some(position);
            self.centreline
                .push((centreline_m, index, f64::from(point.y)));
        }
        let knots = knots_start..self.knots.len();
        set_span_frame_laterals(&mut self.knots[knots.clone()]);
        let sides = self.push_bands(edge, key.end);
        self.arms.push(ArmRecord {
            key,
            knots,
            centreline: centreline_start..self.centreline.len(),
            sides,
            max_station_m: 0.5 * station_m,
            crossable,
            attached,
            structure: edge.class != EdgeClass::Standard,
            bridge: edge.class == EdgeClass::Bridge,
        });
    }

    // The profile split at the centreline into two sides listed outward. A band across the
    // centreline (a footpath) belongs to both.
    fn push_bands(&mut self, edge: &Edge, end: EdgeEnd) -> [(Range<usize>, usize); 2] {
        let profile = RoadProfile::for_edge(edge);
        let bands = profile.bands();
        let arm_band = |band: &ProfileBand, outer_m: f32| ArmBand {
            kind: band.kind,
            level: band.level,
            outer_m: f64::from(outer_m),
        };
        let positive = self.bands.len();
        let outward = bands.iter().filter(|band| band.end_m > 0.0);
        self.bands
            .extend(outward.map(|band| arm_band(band, band.end_m)));
        let negative = self.bands.len();
        let outward = bands.iter().rev().filter(|band| band.start_m < 0.0);
        self.bands
            .extend(outward.map(|band| arm_band(band, -band.start_m)));
        let side = |range: Range<usize>| {
            let listed = self.bands[range.clone()].iter();
            let core = listed.take_while(|band| band.kind == BandKind::Carriageway);
            (range, core.count().max(1))
        };
        let (positive, negative) = (side(positive..negative), side(negative..self.bands.len()));
        match end {
            EdgeEnd::Start => [positive, negative],
            EdgeEnd::End => [negative, positive],
        }
    }

    /// Whether the node's piece stands on the terrain: a standard edge meets the node. Where only
    /// bridges and tunnels meet, the piece is part of the structure: no skirts, no terrain seams,
    /// no coverage (`TERRAIN-07-C`).
    pub(super) fn grounded(&self) -> bool {
        self.grounded
    }

    /// Number of arms.
    pub(super) fn len(&self) -> usize {
        self.arms.len()
    }

    /// Key of arm `index`, in ascending key order.
    pub(super) fn key(&self, index: usize) -> ArmKey {
        self.arms[index].key
    }

    /// Layout input of arm `index`.
    pub(super) fn input(&self, index: usize) -> ArmInput<'_> {
        let arm = &self.arms[index];
        let [left, right] = arm.sides.clone().map(|(bands, core_bands)| ArmSide {
            bands: &self.bands[bands],
            core_bands,
        });
        ArmInput {
            key: arm.key,
            knots: &self.knots[arm.knots.clone()],
            left,
            right,
            max_station_m: arm.max_station_m,
            crossable: arm.crossable,
            attached: arm.attached,
        }
    }

    /// Whether arm `index` is a bridge or tunnel: at a grounded node its mouth row closes the
    /// grounded chains with a wall run (`TERRAIN-07-C`, `docs/terrain.md` T22 3 (a)).
    pub(super) fn is_structure(&self, index: usize) -> bool {
        self.arms[index].structure
    }

    /// Whether arm `index` is a bridge, whose span hangs the abutment face below its mouth row
    /// where a grounded piece shares it ([`super::PieceMesh::for_each_abutment_triangle`]).
    pub(super) fn is_bridge(&self, index: usize) -> bool {
        self.arms[index].bridge
    }

    /// Whether arm `index` is a footpath attached to a road node's sidewalk ring (D5), whose
    /// node-piece ribbon and mouth stand at sidewalk height.
    pub(super) fn is_attached(&self, index: usize) -> bool {
        self.arms[index].attached
    }

    /// The level arm `index`'s band of `level` stands at in the node: an attached footpath at
    /// sidewalk height (D5), any other band at its own.
    pub(super) fn level_at_node(&self, index: usize, level: Level) -> Level {
        if self.arms[index].attached {
            Level::Raised
        } else {
            level
        }
    }

    /// Lays the node out; arm `i` of the input is arm `i` here. A bend's default crossing that
    /// does not fit on its legacy-ordered arm, so that arm is too short or its corner does not
    /// fit (the two errors whose budgets the crossing's depth enters), moves to the other arm,
    /// as the pass-through rule does; the arms then say so too ([`Self::input`]). Any other
    /// error, or the same on the other arm, fails the node with the first arm's error (D8,
    /// `ROAD-68` open question 13). Node pieces, clips and lanes all lay out through here.
    pub(crate) fn layout(&mut self) -> Result<JunctionLayout, LayoutError> {
        let mut layout = JunctionLayout::empty();
        self.layout_into(&mut layout, &mut ArmsScratch::default())?;
        Ok(layout)
    }

    // `layout` in place, into `layout` and `scratch`'s buffers; on failure `layout` holds a
    // partial layout.
    pub(super) fn layout_into(
        &mut self,
        layout: &mut JunctionLayout,
        scratch: &mut ArmsScratch,
    ) -> Result<(), LayoutError> {
        let error = match self.build_layout(layout, scratch) {
            Ok(()) => return Ok(()),
            Err(error) => error,
        };
        let Some([from, to]) = self.crossing_fallback else {
            return Err(error);
        };
        let first = self.arms[from].key;
        if !matches!(error, LayoutError::ArmTooShort { arm } | LayoutError::CornerDoesNotFit { arm }
            if arm == first)
        {
            return Err(error);
        }
        self.cross_on([from, to], to);
        let built = self.build_layout(layout, scratch);
        if built.is_err() {
            self.cross_on([from, to], from);
        }
        built.map_err(|_| error)
    }

    // Puts a bend's default crossing on `onto` of `[from, to]`; the fallback is the other arm.
    fn cross_on(&mut self, [from, to]: [usize; 2], onto: usize) {
        self.arms[from].crossable = onto == from;
        self.arms[to].crossable = onto == to;
        self.crossing_fallback = (onto == from).then_some([from, to]);
    }

    fn build_layout(
        &self,
        layout: &mut JunctionLayout,
        scratch: &mut ArmsScratch,
    ) -> Result<(), LayoutError> {
        let mut inputs = recycle(std::mem::take(&mut scratch.inputs));
        inputs.extend((0..self.len()).map(|index| self.input(index)));
        let built = layout.rebuild(self.position, &inputs, &mut scratch.layout);
        scratch.inputs = recycle(inputs);
        built
    }

    /// Where horizontal station `station_m` of arm `index` lies on its edge's centreline points
    /// (the physical profile), and the finalized centreline height there: the first segment that
    /// reaches the station, interpolated linearly.
    pub(super) fn centreline_at(&self, index: usize, station_m: f64) -> (CentrelinePosition, f64) {
        let points = &self.centreline[self.arms[index].centreline.clone()];
        let segment = points[1..]
            .partition_point(|&(station, _, _)| station < station_m)
            .min(points.len() - 2);
        let ((from_m, from, from_y), (to_m, to, to_y)) = (points[segment], points[segment + 1]);
        let t = if to_m > from_m {
            (station_m - from_m) / (to_m - from_m)
        } else {
            0.0
        };
        let height_m = from_y * (1.0 - t) + to_y * t;
        let (lo, hi, t) = if from < to {
            (from, to, t)
        } else {
            (to, from, 1.0 - t)
        };
        (CentrelinePosition { lo, hi, t }, height_m)
    }

    /// Height of the node's own centreline point.
    pub(super) fn node_height_m(&self, index: usize) -> f64 {
        self.centreline[self.arms[index].centreline.start].2
    }
}

// `inputs` emptied, on the same allocation, for arms of another lifetime: collecting a vector's
// own `IntoIter` reuses its allocation when the item layouts match (std's in-place collection),
// and these items differ only in lifetime. `filter` cannot change the item's lifetime.
#[allow(clippy::unnecessary_filter_map)]
fn recycle<'a>(inputs: Vec<ArmInput<'_>>) -> Vec<ArmInput<'a>> {
    inputs.into_iter().filter_map(|_| None).collect()
}

// An edge's points walked away from the node at `end`, with their indices.
fn walk_away(points: &[Vector3], end: EdgeEnd) -> impl Iterator<Item = (usize, Vector3)> + '_ {
    let last = points.len().saturating_sub(1);
    (0..points.len()).map(move |walked| {
        let index = match end {
            EdgeEnd::Start => walked,
            EdgeEnd::End => last - walked,
        };
        (index, points[index])
    })
}

// Span-frame laterals: left of the centred secant at interior knots, of the first and last
// segment at the ends.
fn set_span_frame_laterals(knots: &mut [ArmKnot]) {
    let count = knots.len();
    if count < 2 {
        return;
    }
    for index in 0..count {
        let (from, to) = (index.saturating_sub(1), (index + 1).min(count - 1));
        let along = knots[to].position - knots[from].position;
        // Divide rather than multiply by the reciprocal: an axis-aligned arm keeps an exact axis
        // (`249 * (1 / 249)` rounds below one), so its runs stay on the span's grid lines.
        let length = along.length();
        knots[index].lateral = if length > 0.0 {
            (along / length).perp()
        } else {
            RoadVec2::ZERO
        };
    }
}

// The live surface edge ends at `node`, sorted by key, each with its direction away from the
// node; `None` for a node merged into another.
fn node_incidents(graph: &RegionGraph, node: u32) -> Option<Vec<(ArmKey, RoadVec2)>> {
    let mut incidents = Vec::with_capacity(4);
    node_incidents_into(graph, node, &mut incidents).then_some(incidents)
}

// `node_incidents` into `incidents`; false for a node merged into another.
fn node_incidents_into(
    graph: &RegionGraph,
    node: u32,
    incidents: &mut Vec<(ArmKey, RoadVec2)>,
) -> bool {
    if graph.get_valid_node(node) != node || node as usize >= graph.node_adjacency_count() {
        return false;
    }
    incidents.clear();
    for &edge_idx in graph.node_adjacency(node) {
        let Some(edge) = graph
            .get_edge(edge_idx)
            .filter(|edge| is_surface_edge(edge))
        else {
            continue;
        };
        let end = if graph.get_valid_node(edge.start_node) == node {
            EdgeEnd::Start
        } else if graph.get_valid_node(edge.end_node) == node {
            EdgeEnd::End
        } else {
            continue;
        };
        if let Some(direction) = incident_direction(edge, end) {
            incidents.push((
                ArmKey {
                    edge: edge_idx,
                    end,
                },
                direction,
            ));
        }
    }
    incidents.sort_by_key(|(key, _)| *key);
    incidents.dedup_by_key(|(key, _)| *key);
    true
}

/// Whether two arm directions away from one node ([`incident_direction`]) are close enough to
/// opposite to be one straight line: the straight-pair test of node layouts, span ends and the
/// profile solve, which [`edges_pass_through`] completes.
pub(crate) fn directions_pass_through(a: RoadVec2, b: RoadVec2) -> bool {
    a.dot(b) <= f64::from(-PASS_THROUGH_DOT_THRESHOLD)
}

/// Whether the two arms of a degree-two node pass through, so their spans share one bisected
/// mouth section and the node has no layout: their directions pass through and their edges have
/// one profile. A road continued by a footpath attaches it to its cap (D5), and one continued by
/// a road of another profile (width, sidewalks or shoulders) is laid out, its straight sectors
/// tapering (D4; `ROAD-68-F` part 2): spans of different profiles share no mouth row. The one
/// pass-through rule of node layouts, span ends and the profile solve.
pub(crate) fn edges_pass_through(a: (&Edge, RoadVec2), b: (&Edge, RoadVec2)) -> bool {
    directions_pass_through(a.1, b.1) && RoadProfile::for_edge(a.0) == RoadProfile::for_edge(b.0)
}

/// How the span of `edge_idx` closes its end row at `node` (`TERRAIN-07-C`, `docs/terrain.md`
/// T22), from the node's arms alone:
/// - a structure's free end (a bridge or tunnel is the node's only surface arm, so the end owns
///   no node piece) is capped, a bridge's down to its slab and a tunnel's to the skirt depth, a
///   road end meeting the ground;
/// - a standard edge passing through into a bridge or tunnel (a pass-through pair, so the two
///   spans share the mouth row without a node piece) closes its chains across the row with a
///   wall run;
/// - a bridge whose mouth row a grounded piece shares (another standard edge meets the node, so
///   the node's piece is grounded or the node passes through into a standard span) hangs its
///   abutment face below the slab's end edge, and a tunnel whose mouth row a grounded piece
///   shares marks it a portal row, near which the graded terrain opens the ground (T22 5 (a));
/// - any other end is closed by the piece it shares the row with.
///
/// `O(degree)`.
pub(super) fn span_end(graph: &RegionGraph, node: u32, edge_idx: usize) -> SpanEnd {
    let Some(incidents) = node_incidents(graph, node) else {
        return SpanEnd::Shared;
    };
    let others = || {
        incidents
            .iter()
            .filter(|(key, _)| key.edge != edge_idx)
            .map(|(key, _)| graph.edge(key.edge).class)
    };
    match graph.edge(edge_idx).class {
        EdgeClass::Standard
            if passes_through(graph, &incidents)
                && others().any(|class| class != EdgeClass::Standard) =>
        {
            SpanEnd::WallRun {
                abutment: others().any(|class| class == EdgeClass::Bridge),
            }
        }
        EdgeClass::Bridge if ends_a_structure(graph, &incidents) => SpanEnd::Cap(StripKind::Fascia),
        EdgeClass::Tunnel if ends_a_structure(graph, &incidents) => SpanEnd::Cap(StripKind::Skirt),
        EdgeClass::Bridge if others().any(|class| class == EdgeClass::Standard) => {
            SpanEnd::Abutment
        }
        EdgeClass::Tunnel if others().any(|class| class == EdgeClass::Standard) => SpanEnd::Portal,
        _ => SpanEnd::Shared,
    }
}

fn ends_a_structure(graph: &RegionGraph, incidents: &[(ArmKey, RoadVec2)]) -> bool {
    matches!(incidents, [(key, _)] if graph.edge(key.edge).class != EdgeClass::Standard)
}

// A pass-through pair ([`edges_pass_through`]), so the node has no layout.
fn passes_through(graph: &RegionGraph, incidents: &[(ArmKey, RoadVec2)]) -> bool {
    matches!(incidents, [(a, da), (b, db)]
        if edges_pass_through((graph.edge(a.edge), *da), (graph.edge(b.edge), *db)))
}

/// The two road arms of a pass-through node, each with whether it carries the node's
/// span-owned crossing (the D8 rule of [`NodeArms::collect`]); `None` for any other node.
pub(super) fn pass_through_crossings(
    graph: &RegionGraph,
    node: u32,
) -> Option<[(ArmKey, bool); 2]> {
    let incidents = node_incidents(graph, node)?;
    let [a, b] = incidents[..] else {
        return None;
    };
    passes_through(graph, &incidents).then(|| {
        let crosses = degree_two_crossings(graph, node, [a, b], true).crosses;
        [(a.0, crosses[0]), (b.0, crosses[1])]
    })
}

// D8 crossings (`ROAD-68-C`). Only car roads with sidewalks cross.
// - Three or more arms: every such arm, unless a node override of `false` removes it.
// - Two arms, bends and pass-through nodes alike: the legacy `ROAD-64` rule
//   ([`degree_two_crossings`]).
// - One arm: none; the sidewalk wraps the round cap.
fn junction_arm_crosses(
    graph: &RegionGraph,
    node: u32,
    incidents: &[(ArmKey, RoadVec2)],
    index: usize,
) -> bool {
    let key = incidents[index].0;
    incidents.len() >= 3
        && has_sidewalks(graph, key)
        && graph.node(node).crosswalk_overrides.get(&key.edge) != Some(&false)
}

// The legacy `ROAD-64` rule at a degree-two node, per arm in input order, and the arms a default
// crossing may move between.
struct DegreeTwoCrossings {
    crosses: [bool; 2],
    // `[from, to]`: the default crossing's arm and the other arm, when that one may cross by
    // default too.
    fallback: Option<[usize; 2]>,
}

// Arms are taken in order of their direction's angle in `(-pi, pi]`, then edge index; an override
// of `true` always crosses, `false` never does, and without an override an arm crosses only if
// no earlier arm did. So a node has exactly one crossing by default, on its first arm, and an
// override of `false` there moves it to the other arm. A pass-through crossing lies on its arm's
// span (`span_owned`: a pass-through node, which has no layout), so that arm must hold the strip
// within the half of its edge the far node leaves it (`JunctionLayout` gives each end at most
// half), or it does not cross. A default crossing on a layout that cannot hold it moves to the
// other arm when that arm has no override ([`NodeArms::layout`]).
fn degree_two_crossings(
    graph: &RegionGraph,
    node: u32,
    arms: [(ArmKey, RoadVec2); 2],
    span_owned: bool,
) -> DegreeTwoCrossings {
    let overrides = &graph.node(node).crosswalk_overrides;
    let fits = |key: ArmKey| {
        !span_owned
            || 0.5 * horizontal_length_m(&graph.edge(key.edge).geometry) >= CROSSWALK_DEPTH_M
    };
    let angle = |(_, direction): (ArmKey, RoadVec2)| direction.y.atan2(direction.x);
    let order = angle(arms[0]).total_cmp(&angle(arms[1]));
    let first = usize::from(order.then(arms[0].0.cmp(&arms[1].0)).is_gt());
    let eligible = |index: usize| has_sidewalks(graph, arms[index].0) && fits(arms[index].0);
    let free = |index: usize| !overrides.contains_key(&arms[index].0.edge);
    let mut crosses = [false; 2];
    let mut crossed = false;
    for index in [first, 1 - first] {
        if !eligible(index) {
            continue;
        }
        crosses[index] = overrides
            .get(&arms[index].0.edge)
            .copied()
            .unwrap_or(!crossed);
        crossed |= crosses[index];
    }
    let by_default = [first, 1 - first]
        .into_iter()
        .find(|&index| crosses[index] && free(index));
    let fallback = by_default
        .map(|from| [from, 1 - from])
        .filter(|&[_, to]| !span_owned && !crosses[to] && eligible(to) && free(to));
    DegreeTwoCrossings { crosses, fallback }
}

fn is_road(graph: &RegionGraph, key: ArmKey) -> bool {
    graph.edge(key.edge).primary_type == TransitType::Road
}

fn has_sidewalks(graph: &RegionGraph, key: ArmKey) -> bool {
    let edge = graph.edge(key.edge);
    edge.primary_type == TransitType::Road
        && edge.allowed_types & TransitFlags::CAR != 0
        && edge.allowed_types & TransitFlags::FOOT != 0
}

// Horizontal length of a control polyline: the station measure of `NodeArms` (exact duplicates
// add nothing).
fn horizontal_length_m(points: &[Vector3]) -> f64 {
    let xz = |point: Vector3| RoadVec2::new(f64::from(point.x), f64::from(point.z));
    let steps = points
        .windows(2)
        .map(|pair| xz(pair[0]).distance(xz(pair[1])));
    steps.sum()
}
