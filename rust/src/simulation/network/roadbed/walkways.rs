// SPDX-License-Identifier: GPL-2.0-only

//! Pedestrian walkways of one node from its junction layout (`ROAD-68-C`): where its sidewalks
//! end, its zebra crossings and the sidewalk corner routes between neighbouring arms.
//!
//! The lane rebuild reads them through [`NodeWalkways::collect`], which lays the node out with the
//! graph-to-layout function node pieces use ([`NodeArms::collect`], [`NodeArms::layout`]). Both
//! read the same graph, so lanes and node pieces agree bitwise whichever of the lane rebuild and
//! the roadbed compile runs first. Nothing here derives geometry of its own:
//!
//! - A sidewalk runs on the centre of its raised walkway, midway between the carriageway edge
//!   (the kerb) and the sidewalk's outer edge, where sidewalk lanes have always run. It ends on
//!   the crossing line of a crossable arm and at the mouth of any other arm.
//! - A crossing is the layout's [`Crosswalk`], with its kerb points on the node plane
//!   ([`crossing_kerbs`], the function node pieces publish them with).
//! - A corner route runs from one arm's sidewalk end back along its walkway to the sector, along
//!   the centre of the sector's walkway (midpoints of its carriageway-edge and outermost boundary
//!   samples) and out along the next arm's walkway to its sidewalk end, so it never leaves the
//!   sidewalk. A terminal's route wraps its round cap.
//!
//! A pass-through node has no layout, but its crossing (D8: one by default) is span-owned
//! ([`pass_through_crossing`]): the crossing arm's sidewalks end on the crossing line, taken with
//! the kerb points from the span's section there, and the node keeps the corner walks of a node
//! without a layout (no corner routes here).
//!
//! `O(degree * arm knots + sector samples)` per node, plus a far-node plan per pass-through
//! crossing; allocates only the returned buffers and the span model's.

use super::arms::{NodeArms, pass_through_crossings};
use super::layout::{
    ArmInput, ArmKey, ArmSide, EdgeEnd, JunctionLayout, NodeKind, SectorBoundary, SectorLayout,
    Side, for_each_run_station,
};
use super::node::crossing_kerbs;
use super::span::{SpanCrossing, pass_through_crossing};
use super::{BandKind, RoadBand, RoadSection, RoadVec2, RoadVec3};
use crate::simulation::network::graph::RegionGraph;
use godot::prelude::Vector3;
use std::ops::Range;

/// Where one sidewalk ends at a node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SidewalkEnd {
    /// Horizontal metres along the arm's centreline from the node.
    pub(crate) station_m: f64,
    /// End point on the walkway centre, at the arm centreline's height there.
    pub(crate) point: RoadVec3,
}

/// A sidewalk corner route between two neighbouring arms, counter-clockwise from `from`'s left
/// sidewalk end to `to`'s right sidewalk end.
pub(crate) struct CornerRoute<'a> {
    /// Arm whose left sidewalk the route leaves.
    pub(crate) from: ArmKey,
    /// Arm whose right sidewalk the route reaches; `from` itself at a terminal.
    pub(crate) to: ArmKey,
    /// Route points, starting and ending exactly at the two sidewalk ends.
    pub(crate) points: &'a [RoadVec3],
}

// Sidewalk ends and crossing of one arm, in the layout's counter-clockwise order.
#[derive(Clone, Copy, Debug)]
struct WalkArm {
    key: ArmKey,
    // Right side, then left; `None` where the side has no sidewalk.
    ends: [Option<SidewalkEnd>; 2],
    kerbs: Option<[Vector3; 2]>,
}

const RIGHT: usize = 0;
const LEFT: usize = 1;

/// The walkways of one node, for the lane rebuild.
pub(crate) struct NodeWalkways {
    // Every layout arm; at a pass-through node only its crossing arms.
    arms: Vec<WalkArm>,
    // False at a pass-through node, which has no layout and no corner routes.
    layout: bool,
    // Corner routes: from-arm and to-arm (layout order) and their points.
    corners: Vec<(usize, usize, Range<usize>)>,
    points: Vec<RoadVec3>,
}

impl NodeWalkways {
    /// Walkways of `node`: its layout's, or a pass-through node's span-owned crossings. `None`
    /// for a pass-through node without a crossing and for the other nodes without a layout: the
    /// cases of [`NodeArms::collect`], a layout that fails (a placement rejection) or a bend or
    /// junction without a solved plane, which leaves it without a node piece too.
    pub(crate) fn collect(graph: &RegionGraph, node: u32) -> Option<Self> {
        let Some(arms) = NodeArms::collect(graph, node) else {
            return Self::pass_through(graph, node);
        };
        let layout = arms.layout().ok()?;
        let plane = match layout.kind {
            NodeKind::Terminal => None,
            NodeKind::Bend | NodeKind::Junction => {
                Some(graph.solved_junction_endpoint_profile_plane(node)?)
            }
        };
        let walk_arms: Vec<WalkArm> = layout
            .arms
            .iter()
            .map(|arm| {
                let input = arms.input(arm.input_index);
                let station_m = arm
                    .crosswalk
                    .map_or(arm.mouth_station_m, |crosswalk| crosswalk.station_m);
                let height_m = arms.centreline_at(arm.input_index, station_m).1;
                let end = |offset_m: Option<f64>| {
                    offset_m.map(|offset_m| {
                        let xz = input.offset_point(station_m, offset_m);
                        SidewalkEnd {
                            station_m,
                            point: RoadVec3::new(xz.x, height_m, xz.y),
                        }
                    })
                };
                WalkArm {
                    key: arm.key,
                    ends: [
                        end(walk_offset(input.right).map(|offset| -offset)),
                        end(walk_offset(input.left)),
                    ],
                    kerbs: arm
                        .crosswalk
                        .zip(plane.as_ref())
                        .map(|(crosswalk, plane)| crossing_kerbs(plane, &crosswalk)),
                }
            })
            .collect();
        let mut walkways = Self {
            corners: Vec::with_capacity(layout.sectors.len()),
            points: Vec::new(),
            arms: walk_arms,
            layout: true,
        };
        for sector in &layout.sectors {
            let (Some(start), Some(end)) = (
                walkways.arms[sector.from_arm].ends[LEFT],
                walkways.arms[sector.to_arm].ends[RIGHT],
            ) else {
                continue;
            };
            let first = walkways.points.len();
            push_corner_route(&mut walkways.points, &layout, &arms, sector, [start, end]);
            walkways
                .corners
                .push((sector.from_arm, sector.to_arm, first..walkways.points.len()));
        }
        Some(walkways)
    }

    // A pass-through node's crossing arms, each with its sidewalk ends and kerbs on the span's
    // section at the crossing line.
    fn pass_through(graph: &RegionGraph, node: u32) -> Option<Self> {
        let arms: Vec<WalkArm> = pass_through_crossings(graph, node)?
            .into_iter()
            .filter(|&(_, crosses)| crosses)
            .filter_map(|(key, _)| Some(span_walk_arm(key, &pass_through_crossing(graph, key)?)))
            .collect();
        (!arms.is_empty()).then_some(Self {
            arms,
            layout: false,
            corners: Vec::new(),
            points: Vec::new(),
        })
    }

    /// Whether the node has a junction layout, whose corner routes join its sidewalks; a
    /// pass-through node's sidewalks join as at a node without one.
    pub(crate) fn has_layout(&self) -> bool {
        self.layout
    }

    /// The end of the sidewalk on the side of arm `key` whose lateral offset is positive
    /// (`positive_lateral`) or negative in its edge's forward frame.
    pub(crate) fn sidewalk_end(&self, key: ArmKey, positive_lateral: bool) -> Option<SidewalkEnd> {
        // At an edge's start the layout's left is the positive lateral side; at its end, the
        // negative one.
        let side = if positive_lateral == (key.end == EdgeEnd::Start) {
            LEFT
        } else {
            RIGHT
        };
        self.arm(key)?.ends[side]
    }

    /// Crossings in the layout's counter-clockwise arm order: each crossable arm's key and its
    /// kerb points, right side first.
    pub(crate) fn crossings(&self) -> impl Iterator<Item = (ArmKey, [Vector3; 2])> + '_ {
        self.arms
            .iter()
            .filter_map(|arm| Some((arm.key, arm.kerbs?)))
    }

    /// Corner routes in the layout's sector order.
    pub(crate) fn corners(&self) -> impl Iterator<Item = CornerRoute<'_>> {
        self.corners.iter().map(|(from, to, range)| CornerRoute {
            from: self.arms[*from].key,
            to: self.arms[*to].key,
            points: &self.points[range.clone()],
        })
    }

    fn arm(&self, key: ArmKey) -> Option<&WalkArm> {
        self.arms.iter().find(|arm| arm.key == key)
    }
}

// A crossing arm of a pass-through node from its span crossing: sidewalk ends on the walkway
// centre of the crossing section, at the centreline height as layout ends are, and the section
// row's kerb vertices.
fn span_walk_arm(key: ArmKey, crossing: &SpanCrossing) -> WalkArm {
    let section = &crossing.section;
    let point = |lateral_m: f64| {
        let xz = section.center_xz + section.lateral_xz * lateral_m;
        RoadVec3::new(xz.x, f64::from(section.center_height_m), xz.y)
    };
    let [negative, positive] = section_walk_laterals(section).map(|lateral_m| {
        lateral_m.map(|lateral_m| SidewalkEnd {
            station_m: crossing.station_m,
            point: point(lateral_m),
        })
    });
    let kerb = |p: RoadVec3| Vector3::new(p.x as f32, p.y as f32, p.z as f32);
    let [negative_kerb, positive_kerb] = crossing.kerbs.map(kerb);
    // Walking away from the node, the right side is the negative lateral one at an edge's start
    // and the positive one at its end.
    let (ends, kerbs) = match key.end {
        EdgeEnd::Start => ([negative, positive], [negative_kerb, positive_kerb]),
        EdgeEnd::End => ([positive, negative], [positive_kerb, negative_kerb]),
    };
    WalkArm {
        key,
        ends,
        kerbs: Some(kerbs),
    }
}

// Signed laterals of a section's walkway centres, negative side first: midway between the
// carriageway edge and the section's outer edge, on a side with a sidewalk band.
fn section_walk_laterals(section: &RoadSection) -> [Option<f64>; 2] {
    let bands = &section.bands;
    let carriageway = |band: &RoadBand| band.kind == BandKind::Carriageway;
    let (Some(first), Some(last)) = (
        bands.iter().position(carriageway),
        bands.iter().rposition(carriageway),
    ) else {
        return [None, None];
    };
    let has_sidewalk = |side: &[RoadBand]| side.iter().any(|band| band.kind == BandKind::Sidewalk);
    let centre = |kerb: f32, outer: f32| 0.5 * (f64::from(kerb) + f64::from(outer));
    [
        has_sidewalk(&bands[..first])
            .then(|| centre(bands[first].lateral_start_m, bands[0].lateral_start_m)),
        has_sidewalk(&bands[last + 1..]).then(|| {
            centre(
                bands[last].lateral_end_m,
                bands[bands.len() - 1].lateral_end_m,
            )
        }),
    ]
}

// Unsigned offset of a side's walkway centre: midway between the carriageway edge and the
// sidewalk's outer edge. `None` without a sidewalk band.
fn walk_offset(side: ArmSide<'_>) -> Option<f64> {
    let outer = side.bands.last()?;
    side.bands[side.core_bands..]
        .iter()
        .any(|band| band.kind == BandKind::Sidewalk)
        .then(|| 0.5 * (side.core_offset() + outer.outer_m))
}

// Appends the corner route of `sector` from its from-arm's left sidewalk end to its to-arm's
// right one. Heights run linearly by horizontal length between the two ends.
fn push_corner_route(
    points: &mut Vec<RoadVec3>,
    layout: &JunctionLayout,
    arms: &NodeArms,
    sector: &SectorLayout,
    [start, end]: [SidewalkEnd; 2],
) {
    let a = arms.input(layout.arms[sector.from_arm].input_index);
    let b = arms.input(layout.arms[sector.to_arm].input_index);
    let boundaries = &layout.boundaries[sector.boundaries.clone()];
    let (edge, outer) = (&boundaries[0], &boundaries[boundaries.len() - 1]);
    let first = points.len();
    let mut push = |xz: RoadVec2| {
        if points[first..]
            .last()
            .is_none_or(|last| RoadVec2::new(last.x, last.z) != xz)
        {
            points.push(RoadVec3::new(xz.x, 0.0, xz.y));
        }
    };
    push(RoadVec2::new(start.point.x, start.point.z));
    // The walkway centre between the two boundary runs, tapered with them (D4).
    let taper_m = edge.taper.or(outer.taper).map(|taper| taper.station_m);
    let walkway = |arm: &ArmInput<'_>, side: Side, station_m: f64| {
        let offset_m =
            0.5 * (edge.run_offset_m(side, station_m) + outer.run_offset_m(side, station_m));
        arm.offset_point(station_m, offset_m)
    };
    // Back along the from-arm's walkway to where the sector's boundaries leave it.
    let a_corner_m = edge.from_station_m.max(outer.from_station_m);
    let mut back = Vec::new();
    for_each_run_station(&a, a_corner_m, start.station_m, taper_m, |station_m, _| {
        back.push(walkway(&a, Side::Left, station_m));
    });
    for &xz in back.iter().rev() {
        push(xz);
    }
    let samples = |boundary: &SectorBoundary| &layout.samples[boundary.samples.clone()];
    for (inner, outer) in samples(edge).iter().zip(samples(outer)) {
        push((*inner + *outer) * 0.5);
    }
    // Out along the to-arm's walkway to its sidewalk end.
    let b_corner_m = edge.to_station_m.max(outer.to_station_m);
    for_each_run_station(&b, b_corner_m, end.station_m, taper_m, |station_m, _| {
        push(walkway(&b, Side::Right, station_m));
    });
    push(RoadVec2::new(end.point.x, end.point.z));
    // The two ends are the arms' sidewalk ends exactly, heights included.
    let route = &mut points[first..];
    let total_m: f64 = route
        .windows(2)
        .map(|pair| RoadVec2::new(pair[0].x - pair[1].x, pair[0].z - pair[1].z).length())
        .sum();
    let mut walked_m = 0.0;
    for index in 0..route.len() {
        if index > 0 {
            let (p, q) = (route[index - 1], route[index]);
            walked_m += RoadVec2::new(p.x - q.x, p.z - q.z).length();
        }
        let t = if total_m > 0.0 {
            walked_m / total_m
        } else {
            0.0
        };
        route[index].y = start.point.y * (1.0 - t) + end.point.y * t;
    }
    route[0] = start.point;
    let last = route.len() - 1;
    if last > 0 {
        route[last] = end.point;
    }
}
