// SPDX-License-Identifier: GPL-2.0-only

//! Node arms from the graph (`ROAD-68-C`): one [`ArmInput`] per live incident edge end, oriented
//! away from the node, the rule that decides which nodes get a layout, and the D8 rule that
//! decides which arms cross ([`pass_through_crossings`] for nodes without one).
//!
//! An arm's knots are its edge's roadbed centreline points (the physical profile, else the
//! control polyline; the polyline spans section) in world XZ, walked away from the node, with
//! exact duplicates dropped. Stations are horizontal metres from the node. Each knot's lateral
//! axis is the span frame's: the centred secant at interior knots and the segment direction at
//! the two ends, rotated a quarter turn to the left of the outward direction. Bands come from the
//! edge's [`RoadProfile`]: for an arm leaving from its edge's start, the layout's left is the
//! profile's positive lateral side; at the edge's end both the axis and the sides swap.
//!
//! Collection is `O(degree * arm points)` and allocates only the returned buffers.

use super::layout::{
    ArmBand, ArmInput, ArmKey, ArmKnot, ArmSide, CROSSWALK_DEPTH_M, EdgeEnd, JunctionLayout,
    LayoutError,
};
use super::profile::RoadProfile;
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
    left: Range<usize>,
    left_core: usize,
    right: Range<usize>,
    right_core: usize,
    max_station_m: f64,
    crossable: bool,
}

/// The arms of one node, owning the knot and band buffers their [`ArmInput`]s borrow.
#[derive(Clone, Debug)]
pub(crate) struct NodeArms {
    /// Node id.
    pub(super) node: u32,
    /// Node position in world XZ; every arm's first knot.
    pub(super) position: RoadVec2,
    arms: Vec<ArmRecord>,
    knots: Vec<ArmKnot>,
    // Per knot: its index into the edge's centreline points and its height there.
    knot_points: Vec<(usize, f64)>,
    bands: Vec<ArmBand>,
}

impl NodeArms {
    /// Arms of `node`, or `None` when the node has no layout in this gate: merged into another
    /// node, no surface arm, a pass-through pair (its spans share one bisected section), a
    /// footpath arm (D5 and footpath-only nodes arrive in `ROAD-68-F`), or a bridge or tunnel
    /// terminal.
    pub(crate) fn collect(graph: &RegionGraph, node: u32) -> Option<Self> {
        let incidents = node_incidents(graph, node)?;
        // Bridge and tunnel ends own their terminals until their portals and abutments
        // (`ROAD-68-F`).
        let structure_end = matches!(incidents.as_slice(), [(key, _)]
            if graph.edge(key.edge).class != EdgeClass::Standard);
        if incidents.is_empty() || passes_through(&incidents) || structure_end {
            return None;
        }
        let pos = graph.node(node).pos;
        let mut arms = Self {
            node,
            position: RoadVec2::new(f64::from(pos.x), f64::from(pos.z)),
            arms: Vec::with_capacity(incidents.len()),
            knots: Vec::new(),
            knot_points: Vec::new(),
            bands: Vec::new(),
        };
        for (index, &(key, _)) in incidents.iter().enumerate() {
            let crossable = arm_crosses(graph, node, &incidents, index);
            arms.push_arm(key, graph.edge(key.edge), crossable);
        }
        Some(arms)
    }

    fn push_arm(&mut self, key: ArmKey, edge: &Edge, crossable: bool) {
        let points = edge_points(edge);
        let last = points.len().saturating_sub(1);
        let knots_start = self.knots.len();
        let mut station_m = 0.0;
        for walked in 0..points.len() {
            let index = match key.end {
                EdgeEnd::Start => walked,
                EdgeEnd::End => last - walked,
            };
            let point = points[index];
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
            self.knot_points.push((index, f64::from(point.y)));
        }
        let knots = knots_start..self.knots.len();
        set_span_frame_laterals(&mut self.knots[knots.clone()]);
        let (left, left_core, right, right_core) = self.push_bands(edge, key.end);
        self.arms.push(ArmRecord {
            key,
            knots,
            left,
            left_core,
            right,
            right_core,
            max_station_m: 0.5 * station_m,
            crossable,
        });
    }

    // The profile split at the centreline into two sides listed outward. A band across the
    // centreline (a footpath) belongs to both.
    fn push_bands(
        &mut self,
        edge: &Edge,
        end: EdgeEnd,
    ) -> (Range<usize>, usize, Range<usize>, usize) {
        let profile = RoadProfile::for_edge(edge);
        let bands = profile.bands();
        let positive = self.bands.len();
        self.bands.extend(
            bands
                .iter()
                .filter(|band| band.end_m > 0.0)
                .map(|band| ArmBand {
                    kind: band.kind,
                    level: band.level,
                    outer_m: f64::from(band.end_m),
                }),
        );
        let negative = self.bands.len();
        self.bands.extend(
            bands
                .iter()
                .rev()
                .filter(|band| band.start_m < 0.0)
                .map(|band| ArmBand {
                    kind: band.kind,
                    level: band.level,
                    outer_m: f64::from(-band.start_m),
                }),
        );
        let (positive, negative) = (positive..negative, negative..self.bands.len());
        let core = |range: &Range<usize>| {
            self.bands[range.clone()]
                .iter()
                .take_while(|band| band.kind == BandKind::Carriageway)
                .count()
                .max(1)
        };
        let (positive_core, negative_core) = (core(&positive), core(&negative));
        match end {
            EdgeEnd::Start => (positive, positive_core, negative, negative_core),
            EdgeEnd::End => (negative, negative_core, positive, positive_core),
        }
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
        ArmInput {
            key: arm.key,
            knots: &self.knots[arm.knots.clone()],
            left: ArmSide {
                bands: &self.bands[arm.left.clone()],
                core_bands: arm.left_core,
            },
            right: ArmSide {
                bands: &self.bands[arm.right.clone()],
                core_bands: arm.right_core,
            },
            max_station_m: arm.max_station_m,
            crossable: arm.crossable,
        }
    }

    /// Lays the node out; arm `i` of the input is arm `i` here.
    pub(crate) fn layout(&self) -> Result<JunctionLayout, LayoutError> {
        let inputs: Vec<_> = (0..self.len()).map(|index| self.input(index)).collect();
        JunctionLayout::build(self.position, &inputs)
    }

    /// Where station `station_m` of arm `index` lies on its edge's centreline points, and the
    /// centreline height there, both by the segment rule of [`ArmInput::offset_point`].
    pub(super) fn centreline_at(&self, index: usize, station_m: f64) -> (CentrelinePosition, f64) {
        let arm = &self.arms[index];
        let knots = &self.knots[arm.knots.clone()];
        let points = &self.knot_points[arm.knots.clone()];
        let segment = knots[1..]
            .partition_point(|knot| knot.station_m < station_m)
            .min(knots.len() - 2);
        let t = (station_m - knots[segment].station_m)
            / (knots[segment + 1].station_m - knots[segment].station_m);
        let ((from, from_y), (to, to_y)) = (points[segment], points[segment + 1]);
        let height_m = from_y * (1.0 - t) + to_y * t;
        let position = if from < to {
            CentrelinePosition {
                lo: from,
                hi: to,
                t,
            }
        } else {
            CentrelinePosition {
                lo: to,
                hi: from,
                t: 1.0 - t,
            }
        };
        (position, height_m)
    }

    /// Height of the node's own centreline point (its first knot).
    pub(super) fn node_height_m(&self, index: usize) -> f64 {
        self.knot_points[self.arms[index].knots.start].1
    }
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
        knots[index].lateral = along.normalize_or_zero().perp();
    }
}

// The live surface edge ends at `node`, sorted by key, each with its direction away from the
// node; `None` for a node merged into another or one with a footpath arm (D5, `ROAD-68-F`).
fn node_incidents(graph: &RegionGraph, node: u32) -> Option<Vec<(ArmKey, RoadVec2)>> {
    if graph.get_valid_node(node) != node || node as usize >= graph.node_adjacency_count() {
        return None;
    }
    let mut incidents: Vec<(ArmKey, RoadVec2)> = Vec::with_capacity(4);
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
            if edge.primary_type == TransitType::Foot {
                return None;
            }
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
    Some(incidents)
}

// A pass-through pair: two arms close enough to opposite that their spans share one bisected
// section and the node has no layout.
fn passes_through(incidents: &[(ArmKey, RoadVec2)]) -> bool {
    matches!(incidents, [(_, a), (_, b)] if a.dot(*b) <= f64::from(-PASS_THROUGH_DOT_THRESHOLD))
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
    passes_through(&incidents).then(|| {
        let crosses = degree_two_crossings(graph, node, [a, b]);
        [(a.0, crosses[0]), (b.0, crosses[1])]
    })
}

// D8 crossings (`ROAD-68-C`): whether arm `index` of `incidents` crosses. Only car roads with
// sidewalks cross.
// - Three or more arms: every such arm, unless a node override of `false` removes it.
// - Two arms, bends and pass-through nodes alike: the legacy `ROAD-64` rule. Arms are taken in
//   order of their direction's angle in `(-pi, pi]`, then edge index; an override of `true`
//   always crosses, `false` never does, and without an override an arm crosses only if no
//   earlier arm did. So a node has exactly one crossing by default, on its first arm, and an
//   override of `false` there moves it to the other arm. A pass-through crossing lies on its
//   arm's span, so that arm must hold the strip within the half of its edge the far node leaves
//   it (`JunctionLayout` gives each end at most half), or it does not cross.
// - One arm: none; the sidewalk wraps the round cap.
fn arm_crosses(
    graph: &RegionGraph,
    node: u32,
    incidents: &[(ArmKey, RoadVec2)],
    index: usize,
) -> bool {
    match *incidents {
        [a, b] => degree_two_crossings(graph, node, [a, b])[index],
        _ => {
            let key = incidents[index].0;
            incidents.len() >= 3
                && has_sidewalks(graph, key)
                && graph.node(node).crosswalk_overrides.get(&key.edge) != Some(&false)
        }
    }
}

// The legacy `ROAD-64` rule at a degree-two node, per arm in input order.
fn degree_two_crossings(
    graph: &RegionGraph,
    node: u32,
    arms: [(ArmKey, RoadVec2); 2],
) -> [bool; 2] {
    let overrides = &graph.node(node).crosswalk_overrides;
    let pass_through = passes_through(&arms);
    let fits = |key: ArmKey| {
        !pass_through
            || 0.5 * horizontal_length_m(edge_points(graph.edge(key.edge))) >= CROSSWALK_DEPTH_M
    };
    let angle = |(_, direction): (ArmKey, RoadVec2)| direction.y.atan2(direction.x);
    let first = match angle(arms[0])
        .total_cmp(&angle(arms[1]))
        .then(arms[0].0.cmp(&arms[1].0))
    {
        std::cmp::Ordering::Greater => 1,
        _ => 0,
    };
    let mut crossable = [false; 2];
    let mut crossed = false;
    for index in [first, 1 - first] {
        let key = arms[index].0;
        if !has_sidewalks(graph, key) || !fits(key) {
            continue;
        }
        crossable[index] = overrides.get(&key.edge).copied().unwrap_or(!crossed);
        crossed |= crossable[index];
    }
    crossable
}

fn has_sidewalks(graph: &RegionGraph, key: ArmKey) -> bool {
    let edge = graph.edge(key.edge);
    edge.primary_type == TransitType::Road
        && edge.allowed_types & TransitFlags::CAR != 0
        && edge.allowed_types & TransitFlags::FOOT != 0
}

// Horizontal length of a centreline: the station measure of `NodeArms` (exact duplicates add
// nothing).
fn horizontal_length_m(points: &[Vector3]) -> f64 {
    points
        .windows(2)
        .map(|pair| {
            RoadVec2::new(f64::from(pair[0].x), f64::from(pair[0].z))
                .distance(RoadVec2::new(f64::from(pair[1].x), f64::from(pair[1].z)))
        })
        .sum()
}
