// SPDX-License-Identifier: GPL-2.0-only

//! Node arms from the graph (`ROAD-68-C`): one [`ArmInput`] per live incident edge end, oriented
//! away from the node, and the rule that decides which nodes get a layout.
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
    ArmBand, ArmInput, ArmKey, ArmKnot, ArmSide, EdgeEnd, JunctionLayout, LayoutError,
};
use super::profile::RoadProfile;
use super::span::{PASS_THROUGH_DOT_THRESHOLD, edge_points, incident_direction, is_surface_edge};
use super::{BandKind, RoadVec2};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, TransitFlags, TransitType};
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
        // A node merged into another has no arms of its own.
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
        let passes_through = matches!(incidents.as_slice(), [(_, a), (_, b)]
            if a.dot(*b) <= f64::from(-PASS_THROUGH_DOT_THRESHOLD));
        // Bridge and tunnel ends own their terminals until their portals and abutments
        // (`ROAD-68-F`).
        let structure_end = matches!(incidents.as_slice(), [(key, _)]
            if graph.edge(key.edge).class != EdgeClass::Standard);
        if incidents.is_empty() || passes_through || structure_end {
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
        let degree = incidents.len();
        for (key, _) in incidents {
            let edge = graph.edge(key.edge);
            let crossable = arm_is_crossable(graph, node, key.edge, edge, degree);
            arms.push_arm(key, edge, crossable);
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

// D8 crossings in `ROAD-68-C`: every car-road arm with sidewalks at a junction of three or more
// arms, and bend arms only where the node forces a crossing. A node override of `false` removes
// one. `ROAD-68-F` moves the pedestrian lanes onto these layout crossings.
fn arm_is_crossable(
    graph: &RegionGraph,
    node: u32,
    edge_idx: usize,
    edge: &Edge,
    degree: usize,
) -> bool {
    let has_sidewalks = edge.primary_type == TransitType::Road
        && edge.allowed_types & TransitFlags::CAR != 0
        && edge.allowed_types & TransitFlags::FOOT != 0;
    let forced = graph.node(node).crosswalk_overrides.get(&edge_idx).copied();
    has_sidewalks
        && match forced {
            Some(force) => force && degree >= 2,
            None => degree >= 3,
        }
}
